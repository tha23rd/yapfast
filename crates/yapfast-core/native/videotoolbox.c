/* H264 pictures on macOS, decoded by VideoToolbox. The libwebrtc archive
 * Yapfast links on the Mac leaves FFmpeg's H264 decoder out, as WebRTC
 * decodes H264 there through VideoToolbox itself, so video.c and decode.c
 * decode it here: synchronously, on the caller's thread, on the hardware
 * decoder where there is one. VideoToolbox gives bi-planar 4:2:0 pictures,
 * which libyuv, from the same archive, makes the I420 the rest of Yapfast
 * takes. */
#include "videotoolbox.h"

#include <CoreFoundation/CoreFoundation.h>
#include <CoreMedia/CoreMedia.h>
#include <CoreVideo/CoreVideo.h>
#include <VideoToolbox/VideoToolbox.h>
#include <stdlib.h>
#include <string.h>

#include "libyuv/convert.h"

/* The widest or tallest picture, as in video.c and decode.c. */
enum { MAX_SIDE = 8192 };
/* H264 reorders by 16 pictures at most; one more comes before the first
 * goes. Streams have a parameter set or two of each kind. */
enum { MAX_HELD = 17, MAX_SETS = 8 };
enum { SPS = 7, PPS = 8, ACCESS_UNIT_DELIMITER = 9 };

struct set {
  uint8_t *data;
  size_t size;
};

struct held {
  CVPixelBufferRef buffer;
  int64_t time;
};

struct yapfast_vt {
  VTDecompressionSessionRef session;
  CMVideoFormatDescriptionRef format;
  int annexb;
  /* The bytes of each NAL unit's length in what VideoToolbox gets. */
  int length_size;
  struct set sets[MAX_SETS];
  int set_count;
  /* Pictures decoded and not yet given, in the order they show. */
  struct held held[MAX_HELD];
  int held_count;
  int delay;
  int draining;
  /* An Annex B access unit, rewritten with lengths. */
  uint8_t *unit;
  size_t unit_capacity;
  /* The last picture given, as I420. */
  uint8_t *planes;
  size_t planes_size;
};

static void release_held(struct yapfast_vt *vt) {
  for (int index = 0; index < vt->held_count; index++) CVPixelBufferRelease(vt->held[index].buffer);
  vt->held_count = 0;
}

static void end_session(struct yapfast_vt *vt) {
  if (vt->session) {
    VTDecompressionSessionInvalidate(vt->session);
    CFRelease(vt->session);
    vt->session = NULL;
  }
  if (vt->format) {
    CFRelease(vt->format);
    vt->format = NULL;
  }
}

void yapfast_vt_free(struct yapfast_vt *vt) {
  if (!vt) return;
  end_session(vt);
  release_held(vt);
  for (int index = 0; index < vt->set_count; index++) free(vt->sets[index].data);
  free(vt->unit);
  free(vt->planes);
  free(vt);
}

/* Keeps each picture decoded, in the order they show; one without a time
 * goes last. Decoding is synchronous, so this runs before
 * VTDecompressionSessionDecodeFrame returns. */
static void decoded(void *opaque, void *frame, OSStatus status, VTDecodeInfoFlags flags,
                    CVImageBufferRef image, CMTime time, CMTime duration) {
  (void)frame, (void)flags, (void)duration;
  struct yapfast_vt *vt = opaque;
  if (status != noErr || !image || vt->held_count == MAX_HELD) return;
  int64_t shows = CMTIME_IS_NUMERIC(time) ? time.value : YAPFAST_VT_NO_TIME;
  int at = vt->held_count;
  while (shows != YAPFAST_VT_NO_TIME && at > 0 && vt->held[at - 1].time != YAPFAST_VT_NO_TIME &&
         vt->held[at - 1].time > shows) {
    vt->held[at] = vt->held[at - 1];
    at--;
  }
  vt->held[at] = (struct held){CVPixelBufferRetain(image), shows};
  vt->held_count++;
}

static int kind(const struct set *set) {
  return set->data[0] & 0x1f;
}

/* Whether there are parameter sets of both kinds to decode with. */
static int has_sets(const struct yapfast_vt *vt) {
  int kinds = 0;
  for (int index = 0; index < vt->set_count; index++) {
    kinds |= kind(&vt->sets[index]) == SPS ? 1 : kind(&vt->sets[index]) == PPS ? 2 : 0;
  }
  return kinds == 3;
}

/* Keeps a parameter set: 1 when it's new, 0 when it was kept already, -1
 * without memory. With `replace`, it takes the place of the one of its kind. */
static int keep_set(struct yapfast_vt *vt, const uint8_t *data, size_t size, int replace) {
  int at = vt->set_count;
  for (int index = 0; index < vt->set_count; index++) {
    const struct set *set = &vt->sets[index];
    if (set->size == size && !memcmp(set->data, data, size)) return 0;
    if (replace && kind(set) == (data[0] & 0x1f)) at = index;
  }
  if (at == MAX_SETS) return 0;
  uint8_t *copy = malloc(size);
  if (!copy) return -1;
  memcpy(copy, data, size);
  if (at < vt->set_count) {
    free(vt->sets[at].data);
  } else {
    vt->set_count++;
  }
  vt->sets[at] = (struct set){copy, size};
  return 1;
}

/* A session for the parameter sets kept, in place of any before. */
static int start_session(struct yapfast_vt *vt) {
  end_session(vt);
  const uint8_t *sets[MAX_SETS];
  size_t sizes[MAX_SETS];
  size_t count = 0;
  /* Sequence parameter sets first. */
  for (int pass = SPS; pass <= PPS; pass++) {
    for (int index = 0; index < vt->set_count; index++) {
      if (kind(&vt->sets[index]) != pass) continue;
      sets[count] = vt->sets[index].data;
      sizes[count++] = vt->sets[index].size;
    }
  }
  if (CMVideoFormatDescriptionCreateFromH264ParameterSets(kCFAllocatorDefault, count, sets, sizes,
                                                          vt->length_size, &vt->format) != noErr) {
    vt->format = NULL;
    return -1;
  }
  CMVideoDimensions size = CMVideoFormatDescriptionGetDimensions(vt->format);
  if (size.width <= 0 || size.height <= 0 || size.width > MAX_SIDE || size.height > MAX_SIDE) {
    end_session(vt);
    return -1;
  }
  /* Bi-planar 4:2:0 in the stream's own range; libyuv takes it from there. */
  int32_t types[2] = {kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                      kCVPixelFormatType_420YpCbCr8BiPlanarFullRange};
  CFNumberRef numbers[2] = {CFNumberCreate(NULL, kCFNumberSInt32Type, &types[0]),
                            CFNumberCreate(NULL, kCFNumberSInt32Type, &types[1])};
  CFArrayRef accepted =
      numbers[0] && numbers[1]
          ? CFArrayCreate(NULL, (const void **)numbers, 2, &kCFTypeArrayCallBacks)
          : NULL;
  const void *keys[1] = {kCVPixelBufferPixelFormatTypeKey};
  const void *values[1] = {accepted};
  CFDictionaryRef attributes =
      accepted ? CFDictionaryCreate(NULL, keys, values, 1, &kCFTypeDictionaryKeyCallBacks,
                                    &kCFTypeDictionaryValueCallBacks)
               : NULL;
  for (int index = 0; index < 2; index++) {
    if (numbers[index]) CFRelease(numbers[index]);
  }
  if (accepted) CFRelease(accepted);
  if (!attributes) {
    end_session(vt);
    return -1;
  }
  VTDecompressionOutputCallbackRecord callback = {decoded, vt};
  OSStatus made =
      VTDecompressionSessionCreate(NULL, vt->format, NULL, attributes, &callback, &vt->session);
  CFRelease(attributes);
  if (made != noErr) {
    vt->session = NULL;
    end_session(vt);
    return -1;
  }
  return 0;
}

struct yapfast_vt *yapfast_vt_new(const uint8_t *record, size_t record_size, int delay) {
  struct yapfast_vt *vt = calloc(1, sizeof *vt);
  if (!vt) return NULL;
  vt->delay = delay < 0 ? 0 : delay < MAX_HELD ? delay : MAX_HELD - 1;
  vt->length_size = 4;
  if (!record || record_size == 0 || record[0] != 1) {
    vt->annexb = 1;
    if (record && record_size > 0) yapfast_vt_send(vt, record, record_size, YAPFAST_VT_NO_TIME);
    return vt;
  }
  /* An avcC box: its version, profile and level, the size of each NAL
   * unit's length, then the sequence and the picture parameter sets, each
   * kind counted and each set after its 16-bit size. */
  if (record_size < 7) goto failed;
  vt->length_size = (record[4] & 3) + 1;
  size_t at = 5;
  for (int pass = SPS; pass <= PPS; pass++) {
    if (at >= record_size) goto failed;
    int count = pass == SPS ? record[at] & 0x1f : record[at];
    at++;
    for (int index = 0; index < count; index++) {
      if (at + 2 > record_size) goto failed;
      size_t size = (size_t)record[at] << 8 | record[at + 1];
      at += 2;
      if (size == 0 || size > record_size - at) goto failed;
      if (keep_set(vt, record + at, size, 0) < 0) goto failed;
      at += size;
    }
  }
  if (!has_sets(vt) || start_session(vt) < 0) goto failed;
  return vt;
failed:
  yapfast_vt_free(vt);
  return NULL;
}

/* The NAL unit after the next start code from `*at`, or NULL; `*at` moves to
 * the start code after it. */
static const uint8_t *next_nal(const uint8_t *data, size_t size, size_t *at, size_t *nal_size) {
  size_t start = *at;
  while (start + 3 <= size && !(data[start] == 0 && data[start + 1] == 0 && data[start + 2] == 1)) {
    start++;
  }
  if (start + 3 > size) return NULL;
  start += 3;
  size_t end = start;
  while (end + 3 <= size && !(data[end] == 0 && data[end + 1] == 0 && data[end + 2] == 1)) end++;
  if (end + 3 > size) end = size;
  *at = end;
  /* The zero a four-byte start code begins with, and any padding. */
  while (end > start && data[end - 1] == 0) end--;
  *nal_size = end - start;
  return data + start;
}

/* Keeps an Annex B access unit's parameter sets, starting a session when
 * they're new, and rewrites the rest with lengths: their size, 0 for none, or
 * -1. */
static long rewrite(struct yapfast_vt *vt, const uint8_t *data, size_t size) {
  int changed = 0;
  size_t written = 0, at = 0, nal_size;
  const uint8_t *nal;
  while ((nal = next_nal(data, size, &at, &nal_size))) {
    if (nal_size == 0) continue;
    int type = nal[0] & 0x1f;
    if (type == SPS || type == PPS) {
      int kept = keep_set(vt, nal, nal_size, 1);
      if (kept < 0) return -1;
      changed |= kept;
      continue;
    }
    if (type == ACCESS_UNIT_DELIMITER) continue;
    if (written + 4 + nal_size > vt->unit_capacity) {
      size_t capacity = (written + 4 + nal_size) * 2;
      uint8_t *grown = realloc(vt->unit, capacity);
      if (!grown) return -1;
      vt->unit = grown;
      vt->unit_capacity = capacity;
    }
    uint8_t *length = vt->unit + written;
    length[0] = (uint8_t)(nal_size >> 24);
    length[1] = (uint8_t)(nal_size >> 16);
    length[2] = (uint8_t)(nal_size >> 8);
    length[3] = (uint8_t)nal_size;
    memcpy(length + 4, nal, nal_size);
    written += 4 + nal_size;
  }
  if ((changed || !vt->session) && has_sets(vt) && start_session(vt) < 0) return -1;
  return (long)written;
}

int yapfast_vt_send(struct yapfast_vt *vt, const uint8_t *data, size_t size, int64_t time) {
  const uint8_t *unit = data;
  size_t unit_size = size;
  if (vt->annexb) {
    long written = rewrite(vt, data, size);
    if (written <= 0) return (int)written;
    unit = vt->unit;
    unit_size = (size_t)written;
  }
  if (!vt->session || unit_size == 0) return -1;
  CMBlockBufferRef block = NULL;
  /* The block borrows the bytes, which outlive the synchronous decode. */
  if (CMBlockBufferCreateWithMemoryBlock(NULL, (void *)unit, unit_size, kCFAllocatorNull, NULL, 0,
                                         unit_size, 0, &block) != kCMBlockBufferNoErr) {
    return -1;
  }
  CMSampleTimingInfo timing = {
      .duration = kCMTimeInvalid,
      .presentationTimeStamp = time == YAPFAST_VT_NO_TIME ? kCMTimeInvalid : CMTimeMake(time, 1),
      .decodeTimeStamp = kCMTimeInvalid,
  };
  CMSampleBufferRef sample = NULL;
  OSStatus made =
      CMSampleBufferCreateReady(NULL, block, vt->format, 1, 1, &timing, 1, &unit_size, &sample);
  CFRelease(block);
  if (made != noErr) return -1;
  OSStatus status = VTDecompressionSessionDecodeFrame(vt->session, sample, 0, NULL, NULL);
  VTDecompressionSessionWaitForAsynchronousFrames(vt->session);
  CFRelease(sample);
  /* A session can stop working, as when the Mac sleeps; a new one decodes
   * from the next keyframe. */
  if (status == kVTInvalidSessionErr) start_session(vt);
  return status == noErr ? 0 : -1;
}

/* Makes a picture VideoToolbox decoded I420, in the decoder's own planes. */
static int convert(struct yapfast_vt *vt, CVPixelBufferRef buffer,
                   struct yapfast_vt_picture *picture) {
  OSType type = CVPixelBufferGetPixelFormatType(buffer);
  if ((type != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange &&
       type != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange) ||
      CVPixelBufferGetPlaneCount(buffer) != 2) {
    return -2;
  }
  int width = (int)CVPixelBufferGetWidth(buffer), height = (int)CVPixelBufferGetHeight(buffer);
  if (width <= 0 || height <= 0 || width > MAX_SIDE || height > MAX_SIDE) return -2;
  int chroma_width = (width + 1) / 2, chroma_height = (height + 1) / 2;
  size_t luma = (size_t)width * (size_t)height;
  size_t chroma = (size_t)chroma_width * (size_t)chroma_height;
  if (vt->planes_size < luma + 2 * chroma) {
    uint8_t *grown = realloc(vt->planes, luma + 2 * chroma);
    if (!grown) return -1;
    vt->planes = grown;
    vt->planes_size = luma + 2 * chroma;
  }
  uint8_t *y = vt->planes, *u = y + luma, *v = u + chroma;
  if (CVPixelBufferLockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess) {
    return -1;
  }
  int converted = NV12ToI420(
      CVPixelBufferGetBaseAddressOfPlane(buffer, 0), (int)CVPixelBufferGetBytesPerRowOfPlane(buffer, 0),
      CVPixelBufferGetBaseAddressOfPlane(buffer, 1), (int)CVPixelBufferGetBytesPerRowOfPlane(buffer, 1),
      y, width, u, chroma_width, v, chroma_width, width, height);
  CVPixelBufferUnlockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
  if (converted) return -1;
  /* CVBufferCopyAttachment replaces it from macOS 12; Yapfast runs on 11. */
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
  CFTypeRef matrix = CVBufferGetAttachment(buffer, kCVImageBufferYCbCrMatrixKey, NULL);
#pragma clang diagnostic pop
  *picture = (struct yapfast_vt_picture){
      .width = width,
      .height = height,
      .data = {y, u, v},
      .stride = {width, chroma_width, chroma_width},
      .bt709 = matrix ? CFEqual(matrix, kCVImageBufferYCbCrMatrix_ITU_R_709_2) : -1,
      .full_range = type == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
  };
  return 0;
}

int yapfast_vt_receive(struct yapfast_vt *vt, struct yapfast_vt_picture *picture) {
  if (vt->held_count == 0 || (!vt->draining && vt->held_count <= vt->delay)) return 0;
  struct held next = vt->held[0];
  vt->held_count--;
  memmove(vt->held, vt->held + 1, (size_t)vt->held_count * sizeof *vt->held);
  int converted = convert(vt, next.buffer, picture);
  CVPixelBufferRelease(next.buffer);
  if (converted < 0) return converted;
  picture->time = next.time;
  return 1;
}

void yapfast_vt_drain(struct yapfast_vt *vt) {
  vt->draining = 1;
}

void yapfast_vt_flush(struct yapfast_vt *vt) {
  release_held(vt);
  vt->draining = 0;
}
