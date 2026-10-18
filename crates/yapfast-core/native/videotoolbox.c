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
#include <time.h>

#include "libyuv/convert.h"

/* The widest or tallest picture, as in video.c and decode.c. */
enum { MAX_SIDE = 8192 };
/* H264 reorders by 16 pictures at most; one more comes before the first
 * goes. */
enum { MAX_HELD = 17 };
/* How many pictures to hold back when neither the caller nor the stream
 * says: as many as x264's B-frames reorder by. A stream that reorders by
 * more raises it as it plays, a picture lost each time. */
enum { GUESSED_DELAY = 2 };
/* H264's ids of sequence and picture parameter sets, and the longest set
 * kept: real ones are far shorter, and this way a stream can't make a
 * decoder keep more than about a megabyte of them. */
enum { SPS_IDS = 32, PPS_IDS = 256, MAX_SET = 4096 };
/* The most macroblocks a picture of H264 has, at level 6.2. */
enum { MAX_MACROBLOCKS = 139264 };
/* How many sessions a stream can start in a second, as its parameter sets
 * change or a session stops: each takes a while to make, and a stream could
 * otherwise ask for one with every picture. */
enum { MAX_STARTS = 4 };
enum { SLICE = 1, IDR = 5, SPS = 7, PPS = 8, ACCESS_UNIT_DELIMITER = 9 };

/* The profiles whose sequence parameter sets say their chroma format and
 * bit depths, and may carry scaling matrices. */
static const uint8_t HIGH_PROFILES[] = {100, 110, 122, 244, 44,  83,  86,
                                        118, 128, 134, 135, 138, 139, 144};

struct set {
  uint8_t *data;
  uint16_t size;
  /* For a picture parameter set, the sequence one it refers to. */
  uint8_t sps;
};

struct held {
  CVPixelBufferRef buffer;
  int64_t time;
  /* How many IDR pictures came before it: none decoded after one shows
   * before any decoded before it, whatever their times say. */
  uint32_t epoch;
};

/* What an access unit's slices say: the picture parameter set they refer
 * to, whether theirs is an IDR picture, and whether it's intra-coded, every
 * slice I or SI. */
struct slice {
  unsigned pps;
  int idr;
  int intra;
};

/* What a sequence parameter set says: the size coded, in macroblocks, and
 * how many pictures B-frames reorder by, -1 when it doesn't say. */
struct sps_info {
  uint64_t width;
  uint64_t height;
  int reorder;
};

struct yapfast_vt {
  VTDecompressionSessionRef session;
  /* What the session decodes with: a sequence parameter set and every
   * picture one that refers to it, as they were at a generation. */
  CMVideoFormatDescriptionRef format;
  int format_sps;
  uint32_t format_generation;
  int annexb;
  /* The bytes of each NAL unit's length in what VideoToolbox gets. */
  int length_size;
  struct set sps[SPS_IDS];
  struct set pps[PPS_IDS];
  /* Each sequence parameter set's generation, new when it or a picture one
   * that refers to it changes. */
  uint32_t generations[SPS_IDS];
  uint32_t stamps;
  /* No session started since the decoder was made or flushed; one stopped
   * working or couldn't start; and how many started in the second from
   * `window`. */
  int fresh;
  int stopped;
  double window;
  int starts;
  /* Pictures decoded and not yet given, in the order they show. */
  struct held held[MAX_HELD];
  int held_count;
  /* The delay asked for, -1 to take the stream's, and the one held to. */
  int asked;
  int delay;
  int draining;
  uint32_t epoch;
  /* When the last picture given and the one before it show, and how many
   * IDR pictures came before the last. */
  int64_t shown;
  int64_t before;
  uint32_t shown_epoch;
  /* What VideoToolbox said as it gave the last access unit's picture. */
  OSStatus status;
  /* An Annex B access unit, rewritten with lengths. */
  uint8_t *unit;
  size_t unit_capacity;
  /* The last picture given, as I420. */
  uint8_t *planes;
  size_t planes_size;
};

/* A NAL unit's first bits, without its header and the bytes that keep start
 * codes out of it, as many as a parameter set kept has; what's past the end
 * reads as zeros, and `at` past it says so. */
struct bits {
  uint8_t data[MAX_SET];
  size_t size;
  size_t at;
};

/* Takes up to `limit` bytes of the NAL unit, as many as what's read needs. */
static void unescape(struct bits *bits, const uint8_t *nal, size_t size, size_t limit) {
  bits->size = bits->at = 0;
  int zeros = 0;
  for (size_t index = 1; index < size && bits->size < limit; index++) {
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

/* Reads a sequence parameter set: 0 when it's damaged. How many pictures
 * B-frames reorder by is its VUI's max_num_reorder_frames, 0 for the
 * Baseline profile, which has no B-frames, or for pictures that show in the
 * order they're decoded. */
static int read_sps(const uint8_t *data, size_t size, struct sps_info *info) {
  struct bits bits;
  unescape(&bits, data, size, sizeof bits.data);
  unsigned profile = read_bits(&bits, 8);
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
  /* The reference frames and whether frame numbers may skip, the size in
   * macroblocks, its height in pairs of them when pictures may be fields,
   * then direct prediction and the cropping. */
  ue(&bits), read_bits(&bits, 1);
  info->width = (uint64_t)ue(&bits) + 1;
  info->height = (uint64_t)ue(&bits) + 1;
  if (!read_bits(&bits, 1)) {
    info->height *= 2;
    read_bits(&bits, 1);
  }
  read_bits(&bits, 1);
  if (read_bits(&bits, 1)) ue(&bits), ue(&bits), ue(&bits), ue(&bits);
  if (bits.at > bits.size * 8) return 0;
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
  info->reorder = profile == 66 ? 0 : bits.at <= bits.size * 8 ? said : -1;
  return 1;
}

/* Reads which picture parameter set a slice refers to and whether it's
 * intra-coded, I or SI: 0 when it's damaged. */
static int read_slice(const uint8_t *nal, size_t size, unsigned *pps, int *intra) {
  struct bits bits;
  unescape(&bits, nal, size, 16);
  /* The first macroblock, the slice's type, then the set. */
  ue(&bits);
  unsigned type = ue(&bits) % 5;
  *pps = ue(&bits);
  *intra = type == 2 || type == 4;
  return bits.at <= bits.size * 8 && *pps < PPS_IDS;
}

/* A clock for how often sessions start. */
static double now(void) {
  struct timespec moment;
  clock_gettime(CLOCK_MONOTONIC, &moment);
  return (double)moment.tv_sec + (double)moment.tv_nsec / 1e9;
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
  vt->format_sps = -1;
}

void yapfast_vt_free(struct yapfast_vt *vt) {
  if (!vt) return;
  end_session(vt);
  release_held(vt);
  for (int id = 0; id < SPS_IDS; id++) free(vt->sps[id].data);
  for (int id = 0; id < PPS_IDS; id++) free(vt->pps[id].data);
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
  while (shows != YAPFAST_VT_NO_TIME && at > 0 && vt->held[at - 1].epoch == vt->epoch &&
         vt->held[at - 1].time != YAPFAST_VT_NO_TIME && vt->held[at - 1].time > shows) {
    vt->held[at] = vt->held[at - 1];
    at--;
  }
  vt->held[at] = (struct held){CVPixelBufferRetain(image), shows, vt->epoch};
  vt->held_count++;
}

/* Keeps a parameter set under its id, in place of the one before: 0, or 1
 * when it's left out, as too long or damaged, with an id H264 doesn't have,
 * or for pictures larger than Yapfast decodes or H264 has, however little of
 * them shows; -1 without memory. */
static int keep_set(struct yapfast_vt *vt, const uint8_t *data, size_t size) {
  if (size > MAX_SET) return 1;
  int kind = data[0] & 0x1f;
  struct bits bits;
  unescape(&bits, data, size, 8);
  /* A sequence parameter set's id comes after its profile and level; a
   * picture one's first, then the sequence one's it refers to. */
  if (kind == SPS) read_bits(&bits, 24);
  unsigned id = ue(&bits), sps = kind == PPS ? ue(&bits) : 0;
  if (bits.at > bits.size * 8 || id >= (kind == SPS ? SPS_IDS : PPS_IDS) || sps >= SPS_IDS) {
    return 1;
  }
  struct set *set = kind == SPS ? &vt->sps[id] : &vt->pps[id];
  if (set->data && set->size == size && !memcmp(set->data, data, size)) return 0;
  struct sps_info info;
  if (kind == SPS && (!read_sps(data, size, &info) || info.width * 16 > MAX_SIDE ||
                      info.height * 16 > MAX_SIDE || info.width * info.height > MAX_MACROBLOCKS)) {
    return 1;
  }
  uint8_t *copy = malloc(size);
  if (!copy) return -1;
  memcpy(copy, data, size);
  /* Formats made with the sequence parameter set the picture one referred
   * to are out of date, as are the new one's. */
  if (kind == PPS && set->data) vt->generations[set->sps] = ++vt->stamps;
  free(set->data);
  *set = (struct set){copy, (uint16_t)size, (uint8_t)sps};
  vt->generations[kind == SPS ? id : sps] = ++vt->stamps;
  return 0;
}

/* What VideoToolbox decodes with for a sequence parameter set and every
 * picture one that refers to it, as a stream may give B-pictures a set of
 * their own; NULL when it isn't of a size Yapfast decodes. */
static CMVideoFormatDescriptionRef make_format(const struct yapfast_vt *vt, unsigned sps) {
  const uint8_t *sets[1 + PPS_IDS] = {vt->sps[sps].data};
  size_t sizes[1 + PPS_IDS] = {vt->sps[sps].size};
  size_t count = 1;
  for (int id = 0; id < PPS_IDS; id++) {
    if (!vt->pps[id].data || vt->pps[id].sps != sps) continue;
    sets[count] = vt->pps[id].data;
    sizes[count++] = vt->pps[id].size;
  }
  CMVideoFormatDescriptionRef format = NULL;
  if (CMVideoFormatDescriptionCreateFromH264ParameterSets(kCFAllocatorDefault, count, sets, sizes,
                                                          vt->length_size, &format) != noErr) {
    return NULL;
  }
  CMVideoDimensions size = CMVideoFormatDescriptionGetDimensions(format);
  if (size.width <= 0 || size.height <= 0 || size.width > MAX_SIDE || size.height > MAX_SIDE) {
    CFRelease(format);
    return NULL;
  }
  return format;
}

/* A session for the format; when it can't start, the format goes and the
 * decoder waits to start one again. */
static int start_session(struct yapfast_vt *vt) {
  vt->fresh = 0;
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
  VTDecompressionOutputCallbackRecord callback = {decoded, vt};
  OSStatus made = attributes ? VTDecompressionSessionCreate(NULL, vt->format, NULL, attributes,
                                                            &callback, &vt->session)
                             : kVTAllocationFailedErr;
  if (attributes) CFRelease(attributes);
  if (made != noErr) {
    vt->session = NULL;
    end_session(vt);
    vt->stopped = 1;
    return -1;
  }
  vt->stopped = 0;
  return 0;
}

/* Whether the stream may start another session this second. */
static int may_start(struct yapfast_vt *vt) {
  double moment = now();
  if (moment - vt->window >= 1) {
    vt->window = moment;
    vt->starts = 0;
  }
  if (vt->starts == MAX_STARTS) return 0;
  vt->starts++;
  return 1;
}

/* Readies a session for the parameter sets `slice` refers to: the
 * sequence one and every picture one that refers to it. When they changed,
 * a new session starts at an IDR picture, as the pictures before one may
 * refer to ones this session decoded, so it goes on until then; and after
 * one stopped working, at an intra-coded picture too. VideoToolbox says a
 * session can take a format of new ids of the same size, but then decodes
 * nothing. Past MAX_STARTS a second the session there is goes on, or the
 * unit isn't decoded; the first after the decoder is made or flushed is
 * free. -1 without the sets or a session. */
static int prepare(struct yapfast_vt *vt, const struct slice *slice) {
  const struct set *pps = &vt->pps[slice->pps];
  if (!pps->data || !vt->sps[pps->sps].data) return -1;
  int sps = pps->sps;
  if (vt->session) {
    if (vt->format_sps == sps && vt->format_generation == vt->generations[sps]) return 0;
    if (!slice->idr || !may_start(vt)) return 0;
    end_session(vt);
  } else if (!vt->fresh && ((vt->stopped && !slice->idr && !slice->intra) || !may_start(vt))) {
    return -1;
  }
  vt->format = make_format(vt, (unsigned)sps);
  if (!vt->format) return -1;
  vt->format_sps = sps;
  vt->format_generation = vt->generations[sps];
  /* Without a delay from the caller, the stream's, never less than it was:
   * pictures may have shown it was more. */
  struct sps_info info;
  if (vt->asked < 0 && read_sps(vt->sps[sps].data, vt->sps[sps].size, &info)) {
    int delay = info.reorder < 0 ? GUESSED_DELAY : info.reorder;
    if (delay > vt->delay) vt->delay = delay;
  }
  return start_session(vt);
}

/* Keeps the parameter sets of an avcC box: its version, profile and level,
 * the size of each NAL unit's length, then the sequence and the picture
 * parameter sets, each kind counted and each set after its 16-bit size. -1
 * when it's damaged or without memory. */
static int read_record(struct yapfast_vt *vt, const uint8_t *record, size_t size) {
  if (size < 7) return -1;
  /* Annex B access units are rewritten with four bytes, whatever the box
   * says; another size takes a new format. */
  int length_size = (record[4] & 3) + 1;
  if (!vt->annexb && length_size != vt->length_size) {
    vt->length_size = length_size;
    vt->format_sps = -1;
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
  vt->shown = vt->before = YAPFAST_VT_NO_TIME;
  vt->format_sps = -1;
  vt->fresh = 1;
  vt->length_size = 4;
  vt->annexb = !record || record_size == 0 || record[0] != 1;
  if (record && yapfast_vt_configure(vt, record, record_size) < 0) goto failed;
  if (!vt->annexb) {
    /* An MP4 file's parameter sets come first, so ones that can't be
     * decoded fail now: a session for the first picture parameter set that
     * makes one. */
    int started = 0;
    for (unsigned id = 0; id < PPS_IDS && !started; id++) {
      struct slice first = {.pps = id, .idr = 1, .intra = 1};
      started = vt->pps[id].data && prepare(vt, &first) == 0;
    }
    if (!started) goto failed;
  }
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

/* Keeps the parameter sets among an access unit's NAL units after their
 * lengths, as an MP4 file may change them in its samples, and reads its
 * slices: 1 with any, 0 without, -1 with a damaged one or without memory. A
 * set left out leaves the unit out too, as VideoToolbox would read it. */
static int scan(struct yapfast_vt *vt, const uint8_t *unit, size_t size, struct slice *slice) {
  int found = 0;
  size_t at = 0;
  while (at + (size_t)vt->length_size < size) {
    size_t nal_size = 0;
    for (int index = 0; index < vt->length_size; index++) nal_size = nal_size << 8 | unit[at++];
    if (nal_size == 0 || nal_size > size - at) break;
    const uint8_t *nal = unit + at;
    int type = nal[0] & 0x1f;
    if ((type == SPS || type == PPS) && keep_set(vt, nal, nal_size) != 0) return -1;
    if (type == SLICE || type == IDR) {
      unsigned pps;
      int intra;
      if (!read_slice(nal, nal_size, &pps, &intra)) return -1;
      if (!found) *slice = (struct slice){pps, type == IDR, 1};
      slice->intra &= intra;
      found = 1;
    }
    at += nal_size;
  }
  return found;
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

/* Whether VideoToolbox says the session stopped working. */
static int session_stopped(OSStatus status) {
  return status == kVTInvalidSessionErr || status == kVTVideoDecoderMalfunctionErr;
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
  }
  /* A unit of parameter sets alone, or of other NAL units, has no picture. */
  struct slice slice;
  int found = scan(vt, unit, unit_size, &slice);
  if (found <= 0) return found;
  if (prepare(vt, &slice) < 0) return -1;
  if (slice.idr) vt->epoch++;
  OSStatus status = decode_unit(vt, unit, unit_size, time, cut);
  if (session_stopped(status)) {
    /* A session can stop working, as when the Mac sleeps: a new one decodes
     * the unit again when it can start from it, and otherwise the next unit
     * that can starts one. */
    end_session(vt);
    vt->stopped = 1;
    if ((slice.idr || slice.intra) && prepare(vt, &slice) == 0) {
      status = decode_unit(vt, unit, unit_size, time, cut);
    }
    if (session_stopped(status)) {
      end_session(vt);
      vt->stopped = 1;
    }
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
    /* Pictures from before the last IDR one go at once: none decoded after
     * shows before them. */
    if (vt->held_count == 0 ||
        (!vt->draining && vt->held_count <= vt->delay && vt->held[0].epoch == vt->epoch)) {
      return 0;
    }
    struct held next = vt->held[0];
    vt->held_count--;
    memmove(vt->held, vt->held + 1, (size_t)vt->held_count * sizeof *vt->held);
    if (next.time != YAPFAST_VT_NO_TIME) {
      /* A picture that shows before the last one given, but after the one
       * before that, came too late and is left out: B-frames reorder by more
       * than was held back, so one more is from now on, up to H264's most.
       * One further back is where the file's times jump back, and shows. */
      int along = vt->shown != YAPFAST_VT_NO_TIME && next.epoch == vt->shown_epoch;
      if (along && next.time < vt->shown && next.time > vt->before) {
        CVPixelBufferRelease(next.buffer);
        if (vt->delay < MAX_HELD - 1) vt->delay++;
        continue;
      }
      vt->before = along ? vt->shown : YAPFAST_VT_NO_TIME;
      vt->shown = next.time;
      vt->shown_epoch = next.epoch;
    }
    int converted = convert(vt, next.buffer, picture);
    CVPixelBufferRelease(next.buffer);
    if (converted < 0) return converted;
    picture->time = next.time;
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
  vt->fresh = 1;
  vt->stopped = 0;
  vt->draining = 0;
  vt->shown = vt->before = YAPFAST_VT_NO_TIME;
}
