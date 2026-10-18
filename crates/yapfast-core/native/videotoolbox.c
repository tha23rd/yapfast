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
/* How many pictures to hold back when neither the caller nor the stream
 * says: as many as x264's B-frames reorder by. A stream that reorders by
 * more raises it as it plays, a picture lost each time. */
enum { GUESSED_DELAY = 2 };
enum { SPS = 7, PPS = 8, ACCESS_UNIT_DELIMITER = 9 };

/* The profiles whose sequence parameter sets say their chroma format and
 * bit depths, and may carry scaling matrices. */
static const uint8_t HIGH_PROFILES[] = {100, 110, 122, 244, 44,  83,  86,
                                        118, 128, 134, 135, 138, 139, 144};

struct set {
  uint8_t *data;
  size_t size;
  int kind;
  unsigned id;
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
  /* The parameter sets changed since the session started. */
  int changed;
  /* Pictures decoded and not yet given, in the order they show. */
  struct held held[MAX_HELD];
  int held_count;
  /* The delay asked for, -1 to take the stream's, and the one held to. */
  int asked;
  int delay;
  int draining;
  /* When the last picture given shows. */
  int64_t shown;
  /* What VideoToolbox said as it gave the last access unit's picture. */
  OSStatus status;
  /* An Annex B access unit, rewritten with lengths. */
  uint8_t *unit;
  size_t unit_capacity;
  /* The last picture given, as I420. */
  uint8_t *planes;
  size_t planes_size;
};

/* A NAL unit's bits, without its header and the bytes that keep start codes
 * out of it. A parameter set is far shorter; what's past the end reads as
 * zeros, and `at` past it says so. */
struct bits {
  uint8_t data[256];
  size_t size;
  size_t at;
};

static void unescape(struct bits *bits, const uint8_t *nal, size_t size) {
  bits->size = bits->at = 0;
  int zeros = 0;
  for (size_t index = 1; index < size && bits->size < sizeof bits->data; index++) {
    if (zeros >= 2 && nal[index] == 3) {
      zeros = 0;
      continue;
    }
    zeros = nal[index] == 0 ? zeros + 1 : 0;
    bits->data[bits->size++] = nal[index];
  }
}

static unsigned read_bits(struct bits *bits, int count) {
  unsigned value = 0;
  for (; count > 0; count--, bits->at++) {
    unsigned bit = bits->at < bits->size * 8 ? bits->data[bits->at / 8] >> (7 - bits->at % 8) : 0;
    value = value << 1 | (bit & 1);
  }
  return value;
}

/* An unsigned Exp-Golomb number, ue(v). */
static unsigned ue(struct bits *bits) {
  int zeros = 0;
  while (!read_bits(bits, 1)) {
    if (++zeros == 32) return UINT32_MAX;
  }
  return (1u << zeros) - 1 + read_bits(bits, zeros);
}

/* A signed one, se(v). */
static int se(struct bits *bits) {
  unsigned value = ue(bits);
  return value & 1 ? (int)(value / 2 + 1) : -(int)(value / 2);
}

/* Skips a VUI's buffering parameters: the schedules, their scales, each
 * one's rate, size and constancy, then four fields' lengths. */
static void skip_hrd(struct bits *bits) {
  unsigned count = ue(bits) + 1;
  read_bits(bits, 8);
  for (unsigned index = 0; index < count && index < 32; index++) {
    ue(bits), ue(bits), read_bits(bits, 1);
  }
  read_bits(bits, 20);
}

/* How many pictures B-frames reorder by, as a sequence parameter set says:
 * its VUI's max_num_reorder_frames, 0 for the Baseline profile, which has no
 * B-frames, or for pictures that show in the order they're decoded, or -1
 * when it doesn't say. */
static int reorder_of(const struct set *sps) {
  struct bits bits;
  unescape(&bits, sps->data, sps->size);
  unsigned profile = read_bits(&bits, 8);
  if (profile == 66) return 0;
  /* The constraints, the level and the set's id. */
  read_bits(&bits, 16);
  ue(&bits);
  if (memchr(HIGH_PROFILES, (int)profile, sizeof HIGH_PROFILES)) {
    /* The chroma format, with whether its planes are coded apart, the bit
     * depths, lossless coding, then any scaling lists, each a run of
     * deltas that ends at a zero. */
    unsigned chroma = ue(&bits);
    if (chroma == 3) read_bits(&bits, 1);
    ue(&bits), ue(&bits), read_bits(&bits, 1);
    if (read_bits(&bits, 1)) {
      for (int list = 0; list < (chroma == 3 ? 12 : 8); list++) {
        if (!read_bits(&bits, 1)) continue;
        unsigned last = 8, next = 8;
        for (int index = 0; index < (list < 6 ? 16 : 64) && next; index++) {
          next = (last + (unsigned)se(&bits)) % 256;
          if (next) last = next;
        }
      }
    }
  }
  /* The frame numbers' size, then how pictures count their order, and what
   * that takes. */
  ue(&bits);
  unsigned order = ue(&bits);
  if (order == 0) {
    ue(&bits);
  } else if (order == 1) {
    read_bits(&bits, 1);
    ue(&bits), ue(&bits);
    for (unsigned count = ue(&bits), index = 0; index < count && index < 256; index++) ue(&bits);
  }
  /* The reference frames, whether frame numbers may skip, the size in
   * macroblocks, fields, direct prediction and the cropping. */
  ue(&bits), read_bits(&bits, 1), ue(&bits), ue(&bits);
  if (!read_bits(&bits, 1)) read_bits(&bits, 1);
  read_bits(&bits, 1);
  if (read_bits(&bits, 1)) ue(&bits), ue(&bits), ue(&bits), ue(&bits);
  int said = order == 2 ? 0 : -1;
  if (read_bits(&bits, 1)) {
    /* The VUI: aspect ratio, overscan, signal type, chroma location and
     * timing, the decoders' buffering, then the restrictions, the
     * reordering after four of them. */
    if (read_bits(&bits, 1) && read_bits(&bits, 8) == 255) read_bits(&bits, 32);
    if (read_bits(&bits, 1)) read_bits(&bits, 1);
    if (read_bits(&bits, 1)) {
      read_bits(&bits, 4);
      if (read_bits(&bits, 1)) read_bits(&bits, 24);
    }
    if (read_bits(&bits, 1)) ue(&bits), ue(&bits);
    if (read_bits(&bits, 1)) read_bits(&bits, 32), read_bits(&bits, 32), read_bits(&bits, 1);
    int nal = read_bits(&bits, 1);
    if (nal) skip_hrd(&bits);
    int vcl = read_bits(&bits, 1);
    if (vcl) skip_hrd(&bits);
    if (nal || vcl) read_bits(&bits, 1);
    read_bits(&bits, 1);
    if (read_bits(&bits, 1)) {
      read_bits(&bits, 1);
      ue(&bits), ue(&bits), ue(&bits), ue(&bits);
      unsigned reorder = ue(&bits);
      if (reorder < MAX_HELD) said = (int)reorder;
    }
  }
  return bits.at <= bits.size * 8 ? said : -1;
}

/* A parameter set's id: a picture parameter set's first field, a sequence
 * parameter set's after its profile and level. */
static unsigned set_id(const uint8_t *data, size_t size) {
  struct bits bits;
  unescape(&bits, data, size);
  if ((data[0] & 0x1f) == SPS) read_bits(&bits, 24);
  return ue(&bits);
}

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
 * goes last, and one `cut` from what shows isn't kept. Decoding is
 * synchronous, so this runs before VTDecompressionSessionDecodeFrame
 * returns. */
static void decoded(void *opaque, void *cut, OSStatus status, VTDecodeInfoFlags flags,
                    CVImageBufferRef image, CMTime time, CMTime duration) {
  (void)flags, (void)duration;
  struct yapfast_vt *vt = opaque;
  vt->status = status;
  if (status != noErr || !image || cut || vt->held_count == MAX_HELD) return;
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

/* Whether there are parameter sets of both kinds to decode with. */
static int has_sets(const struct yapfast_vt *vt) {
  int kinds = 0;
  for (int index = 0; index < vt->set_count; index++) {
    kinds |= vt->sets[index].kind == SPS ? 1 : vt->sets[index].kind == PPS ? 2 : 0;
  }
  return kinds == 3;
}

/* Keeps a parameter set in place of the one of its kind and id, the sets
 * changing when it's new; -1 without memory. */
static int keep_set(struct yapfast_vt *vt, const uint8_t *data, size_t size) {
  int kind = data[0] & 0x1f;
  unsigned id = set_id(data, size);
  int at = vt->set_count;
  for (int index = 0; index < vt->set_count; index++) {
    const struct set *set = &vt->sets[index];
    if (set->kind != kind || set->id != id) continue;
    if (set->size == size && !memcmp(set->data, data, size)) return 0;
    at = index;
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
  vt->sets[at] = (struct set){copy, size, kind, id};
  vt->changed = 1;
  return 0;
}

/* A session for the parameter sets kept, in place of any before. */
static int start_session(struct yapfast_vt *vt) {
  end_session(vt);
  vt->changed = 0;
  const uint8_t *sets[MAX_SETS];
  size_t sizes[MAX_SETS];
  size_t count = 0;
  int said = -1;
  /* Sequence parameter sets first. */
  for (int pass = SPS; pass <= PPS; pass++) {
    for (int index = 0; index < vt->set_count; index++) {
      const struct set *set = &vt->sets[index];
      if (set->kind != pass) continue;
      if (pass == SPS && vt->asked < 0) {
        int reorder = reorder_of(set);
        said = reorder > said ? reorder : said;
      }
      sets[count] = set->data;
      sizes[count++] = set->size;
    }
  }
  /* Without a delay from the caller, the stream's, never less than it was:
   * pictures may have shown it was more. */
  if (vt->asked < 0) {
    int delay = said < 0 ? GUESSED_DELAY : said;
    if (delay > vt->delay) vt->delay = delay;
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

/* Keeps the parameter sets of an avcC box: its version, profile and level,
 * the size of each NAL unit's length, then the sequence and the picture
 * parameter sets, each kind counted and each set after its 16-bit size. -1
 * when it's damaged or without memory. */
static int read_record(struct yapfast_vt *vt, const uint8_t *record, size_t size) {
  if (size < 7) return -1;
  /* Annex B access units are rewritten with four bytes, whatever the box says. */
  int length_size = (record[4] & 3) + 1;
  if (!vt->annexb && length_size != vt->length_size) {
    vt->length_size = length_size;
    vt->changed = 1;
  }
  size_t at = 5;
  for (int pass = SPS; pass <= PPS; pass++) {
    if (at >= size) return -1;
    int count = pass == SPS ? record[at] & 0x1f : record[at];
    at++;
    for (int index = 0; index < count; index++) {
      if (at + 2 > size) return -1;
      size_t set_size = (size_t)record[at] << 8 | record[at + 1];
      at += 2;
      if (set_size == 0 || set_size > size - at || keep_set(vt, record + at, set_size) < 0) {
        return -1;
      }
      at += set_size;
    }
  }
  return 0;
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

int yapfast_vt_configure(struct yapfast_vt *vt, const uint8_t *record, size_t record_size) {
  if (record_size > 0 && record[0] == 1) return read_record(vt, record, record_size);
  /* NAL units after start codes, as a stream has them: only their parameter
   * sets count. */
  size_t at = 0, nal_size;
  const uint8_t *nal;
  while ((nal = next_nal(record, record_size, &at, &nal_size))) {
    int type = nal_size > 0 ? nal[0] & 0x1f : 0;
    if ((type == SPS || type == PPS) && keep_set(vt, nal, nal_size) < 0) return -1;
  }
  return 0;
}

struct yapfast_vt *yapfast_vt_new(const uint8_t *record, size_t record_size, int delay) {
  struct yapfast_vt *vt = calloc(1, sizeof *vt);
  if (!vt) return NULL;
  vt->asked = delay;
  vt->delay = delay < 0 ? 0 : delay < MAX_HELD ? delay : MAX_HELD - 1;
  vt->shown = YAPFAST_VT_NO_TIME;
  vt->length_size = 4;
  vt->annexb = !record || record_size == 0 || record[0] != 1;
  if (record && yapfast_vt_configure(vt, record, record_size) < 0) goto failed;
  /* An MP4 file's parameter sets come first, so one that can't be decoded
   * fails now. */
  if (!vt->annexb && (!has_sets(vt) || start_session(vt) < 0)) goto failed;
  return vt;
failed:
  yapfast_vt_free(vt);
  return NULL;
}

/* Keeps an Annex B access unit's parameter sets and rewrites the rest with
 * lengths: their size, 0 for none, or -1. */
static long rewrite(struct yapfast_vt *vt, const uint8_t *data, size_t size) {
  size_t written = 0, at = 0, nal_size;
  const uint8_t *nal;
  while ((nal = next_nal(data, size, &at, &nal_size))) {
    if (nal_size == 0) continue;
    int type = nal[0] & 0x1f;
    if (type == SPS || type == PPS) {
      if (keep_set(vt, nal, nal_size) < 0) return -1;
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
  return (long)written;
}

/* Keeps the parameter sets among an MP4 access unit's NAL units, as a file
 * may change them in its samples; -1 without memory. */
static int keep_sets_in(struct yapfast_vt *vt, const uint8_t *data, size_t size) {
  size_t at = 0;
  while (at + (size_t)vt->length_size < size) {
    size_t nal_size = 0;
    for (int index = 0; index < vt->length_size; index++) nal_size = nal_size << 8 | data[at++];
    if (nal_size == 0 || nal_size > size - at) return 0;
    int type = data[at] & 0x1f;
    if ((type == SPS || type == PPS) && keep_set(vt, data + at, nal_size) < 0) return -1;
    at += nal_size;
  }
  return 0;
}

/* Decodes an access unit with lengths: what VideoToolbox said of it, as it
 * took it or as it gave its picture. */
static OSStatus decode_unit(struct yapfast_vt *vt, const uint8_t *unit, size_t size, int64_t time,
                            int cut) {
  CMBlockBufferRef block = NULL;
  /* The block borrows the bytes, which outlive the synchronous decode. */
  OSStatus status = CMBlockBufferCreateWithMemoryBlock(NULL, (void *)unit, size, kCFAllocatorNull,
                                                       NULL, 0, size, 0, &block);
  if (status != kCMBlockBufferNoErr) return status;
  CMSampleTimingInfo timing = {
      .duration = kCMTimeInvalid,
      .presentationTimeStamp = time == YAPFAST_VT_NO_TIME ? kCMTimeInvalid : CMTimeMake(time, 1),
      .decodeTimeStamp = kCMTimeInvalid,
  };
  CMSampleBufferRef sample = NULL;
  status = CMSampleBufferCreateReady(NULL, block, vt->format, 1, 1, &timing, 1, &size, &sample);
  CFRelease(block);
  if (status != noErr) return status;
  vt->status = noErr;
  status = VTDecompressionSessionDecodeFrame(vt->session, sample, 0, (void *)(uintptr_t)cut, NULL);
  VTDecompressionSessionWaitForAsynchronousFrames(vt->session);
  CFRelease(sample);
  return status != noErr ? status : vt->status;
}

int yapfast_vt_send(struct yapfast_vt *vt, const uint8_t *data, size_t size, int64_t time,
                    int cut) {
  const uint8_t *unit = data;
  size_t unit_size = size;
  if (vt->annexb) {
    long written = rewrite(vt, data, size);
    if (written < 0) return -1;
    unit = vt->unit;
    unit_size = (size_t)written;
  } else if (keep_sets_in(vt, data, size) < 0) {
    return -1;
  }
  /* A session for parameter sets that changed, or in place of one that
   * stopped or couldn't start. */
  if ((vt->changed || !vt->session) && has_sets(vt) && start_session(vt) < 0) return -1;
  if (unit_size == 0) return 0;
  if (!vt->session) return -1;
  OSStatus status = decode_unit(vt, unit, unit_size, time, cut);
  if (status == kVTInvalidSessionErr || status == kVTVideoDecoderMalfunctionErr) {
    /* A session can stop working, as when the Mac sleeps: a new one decodes
     * the unit again, or, when it can't start, the next unit tries. */
    if (start_session(vt) == 0) status = decode_unit(vt, unit, unit_size, time, cut);
    if (status == kVTInvalidSessionErr || status == kVTVideoDecoderMalfunctionErr) end_session(vt);
  }
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
  for (;;) {
    if (vt->held_count == 0 || (!vt->draining && vt->held_count <= vt->delay)) return 0;
    struct held next = vt->held[0];
    vt->held_count--;
    memmove(vt->held, vt->held + 1, (size_t)vt->held_count * sizeof *vt->held);
    /* A picture that shows before the last one given came too late: B-frames
     * reorder by more than was held back, so one more is from now on. */
    if (next.time != YAPFAST_VT_NO_TIME && vt->shown != YAPFAST_VT_NO_TIME &&
        next.time < vt->shown) {
      CVPixelBufferRelease(next.buffer);
      if (vt->delay < MAX_HELD - 1) vt->delay++;
      continue;
    }
    int converted = convert(vt, next.buffer, picture);
    CVPixelBufferRelease(next.buffer);
    if (converted < 0) return converted;
    picture->time = next.time;
    if (next.time != YAPFAST_VT_NO_TIME) vt->shown = next.time;
    return 1;
  }
}

void yapfast_vt_drain(struct yapfast_vt *vt) {
  vt->draining = 1;
}

void yapfast_vt_flush(struct yapfast_vt *vt) {
  release_held(vt);
  /* The next access unit starts a new session, so the pictures after the
   * keyframe sought to can't refer to any from before it. */
  end_session(vt);
  vt->draining = 0;
  vt->shown = YAPFAST_VT_NO_TIME;
}
