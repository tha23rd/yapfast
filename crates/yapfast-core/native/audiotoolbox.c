/* AAC sound on macOS, decoded by AudioToolbox. The libwebrtc archive Yapfast
 * links on the Mac has no FFmpeg decoder for AAC, which most MP4 files'
 * sound is, so video.c decodes it here, a packet at a time, into
 * interleaved floats at the rate and channels the stream has, as FFmpeg's
 * own AudioToolbox decoder does. */
#include "audiotoolbox.h"

#include <AudioToolbox/AudioToolbox.h>
#include <stdlib.h>
#include <string.h>

/* What the converter's input callback says when it has given its packet. */
enum { GIVEN = 1 };
/* The most frames one packet decodes to: 1024 for AAC, 2048 with spectral
 * band replication doubling the rate. */
enum { MOST_FRAMES = 4096 };

/* AAC's layouts, centre first, as AudioToolbox gives them, and the same
 * channels in the order of FFmpeg's decoder, which the rest of Yapfast takes:
 * left and right, the centre, the LFE, then the surrounds, rear ones before
 * side ones, and those above last. */
static const AudioChannelLayoutTag ORDERS[][2] = {
    {kAudioChannelLayoutTag_AAC_3_0, kAudioChannelLayoutTag_MPEG_3_0_A},
    {kAudioChannelLayoutTag_AAC_4_0, kAudioChannelLayoutTag_MPEG_4_0_A},
    {kAudioChannelLayoutTag_AAC_5_0, kAudioChannelLayoutTag_MPEG_5_0_A},
    {kAudioChannelLayoutTag_AAC_5_1, kAudioChannelLayoutTag_MPEG_5_1_A},
    {kAudioChannelLayoutTag_AAC_6_1, kAudioChannelLayoutTag_MPEG_6_1_A},
    {kAudioChannelLayoutTag_AAC_7_1_B, kAudioChannelLayoutTag_WAVE_7_1},
    {kAudioChannelLayoutTag_AAC_7_1_C, kAudioChannelLayoutTag_CICP_14},
};
/* AAC's other 7.1, channel configuration 7, which FFmpeg's encoder writes:
 * FFmpeg's decoder takes its first pair of fronts as left and right and its
 * outer pair as the sides, after the rear, an order no layout tag has. */
static const AudioChannelLabel WIDE[8] = {
    kAudioChannelLabel_LeftCenter,   kAudioChannelLabel_RightCenter,
    kAudioChannelLabel_Center,       kAudioChannelLabel_LFEScreen,
    kAudioChannelLabel_LeftSurround, kAudioChannelLabel_RightSurround,
    kAudioChannelLabel_Left,         kAudioChannelLabel_Right,
};

struct yapfast_aac {
  AudioConverterRef converter;
  int channels;
  float *samples;
};

/* The packet the converter gets next, once. */
struct feed {
  const uint8_t *data;
  UInt32 size;
  int channels;
  int given;
  AudioStreamPacketDescription description;
};

void yapfast_aac_free(struct yapfast_aac *aac) {
  if (!aac) return;
  if (aac->converter) AudioConverterDispose(aac->converter);
  free(aac->samples);
  free(aac);
}

/* An MPEG-4 descriptor's tag and size, as the ESDS box has them. */
static uint8_t *descriptor(uint8_t *out, uint8_t tag, size_t size) {
  *out++ = tag;
  for (int shift = 21; shift > 0; shift -= 7) *out++ = (uint8_t)((size >> shift) | 0x80);
  *out++ = (uint8_t)(size & 0x7f);
  return out;
}

struct yapfast_aac *yapfast_aac_new(const uint8_t *config, size_t config_size, int *rate,
                                    int *channels) {
  if (!config || config_size == 0 || config_size > 1024) return NULL;
  /* AudioToolbox takes the AudioSpecificConfig inside an ES descriptor, as
   * MP4's esds box carries it: the stream, then its decoder's configuration
   * (MPEG-4 audio, an audio stream), then the config itself. */
  uint8_t cookie[64 + 1024];
  uint8_t *at = descriptor(cookie, 0x03, 3 + 5 + 13 + 5 + config_size);
  *at++ = 0, *at++ = 0, *at++ = 0;
  at = descriptor(at, 0x04, 13 + 5 + config_size);
  *at++ = 0x40;
  *at++ = 0x15;
  memset(at, 0, 11);
  at += 11;
  at = descriptor(at, 0x05, config_size);
  memcpy(at, config, config_size);
  at += config_size;
  UInt32 cookie_size = (UInt32)(at - cookie);

  /* The richest form the config describes, as HE-AAC's layers decode to
   * more than the core's rate and channels. */
  AudioFormatInfo info = {.mASBD = {.mFormatID = kAudioFormatMPEG4AAC},
                          .mMagicCookie = cookie,
                          .mMagicCookieSize = cookie_size};
  UInt32 list_size = 0;
  if (AudioFormatGetPropertyInfo(kAudioFormatProperty_FormatList, sizeof info, &info, &list_size) ||
      list_size < sizeof(AudioFormatListItem)) {
    return NULL;
  }
  AudioFormatListItem *list = malloc(list_size);
  if (!list) return NULL;
  OSStatus listed =
      AudioFormatGetProperty(kAudioFormatProperty_FormatList, sizeof info, &info, &list_size, list);
  AudioStreamBasicDescription in = list[0].mASBD;
  free(list);
  if (listed || in.mSampleRate <= 0 || in.mChannelsPerFrame == 0 || in.mChannelsPerFrame > 8) {
    return NULL;
  }
  AudioStreamBasicDescription out = {
      .mSampleRate = in.mSampleRate,
      .mFormatID = kAudioFormatLinearPCM,
      .mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
      .mBytesPerPacket = 4 * in.mChannelsPerFrame,
      .mFramesPerPacket = 1,
      .mBytesPerFrame = 4 * in.mChannelsPerFrame,
      .mChannelsPerFrame = in.mChannelsPerFrame,
      .mBitsPerChannel = 32,
  };
  struct yapfast_aac *aac = calloc(1, sizeof *aac);
  if (!aac) return NULL;
  aac->channels = (int)in.mChannelsPerFrame;
  aac->samples = malloc((size_t)MOST_FRAMES * (size_t)aac->channels * sizeof(float));
  if (!aac->samples || AudioConverterNew(&in, &out, &aac->converter) ||
      AudioConverterSetProperty(aac->converter, kAudioConverterDecompressionMagicCookie,
                                cookie_size, cookie)) {
    yapfast_aac_free(aac);
    return NULL;
  }
  /* Surround in FFmpeg's order, from the layout the stream has; one the
   * converter won't reorder still plays, in AAC's. */
  AudioChannelLayout layout = {0};
  UInt32 layout_size = sizeof layout;
  if (!AudioConverterGetProperty(aac->converter, kAudioConverterOutputChannelLayout, &layout_size,
                                 &layout)) {
    size_t wide_size = sizeof(AudioChannelLayout) + 7 * sizeof(AudioChannelDescription);
    AudioChannelLayout *wide =
        layout.mChannelLayoutTag == kAudioChannelLayoutTag_AAC_7_1 ? calloc(1, wide_size) : NULL;
    if (wide) {
      wide->mChannelLayoutTag = kAudioChannelLayoutTag_UseChannelDescriptions;
      wide->mNumberChannelDescriptions = 8;
      for (int index = 0; index < 8; index++) {
        wide->mChannelDescriptions[index].mChannelLabel = WIDE[index];
      }
      AudioConverterSetProperty(aac->converter, kAudioConverterOutputChannelLayout,
                                (UInt32)wide_size, wide);
      free(wide);
    }
    for (size_t index = 0; index < sizeof ORDERS / sizeof *ORDERS; index++) {
      if (layout.mChannelLayoutTag != ORDERS[index][0]) continue;
      layout = (AudioChannelLayout){.mChannelLayoutTag = ORDERS[index][1]};
      AudioConverterSetProperty(aac->converter, kAudioConverterOutputChannelLayout, sizeof layout,
                                &layout);
      break;
    }
  }
  *rate = (int)in.mSampleRate;
  *channels = aac->channels;
  return aac;
}

/* Gives the converter the packet, then says there's no more for now. */
static OSStatus supply(AudioConverterRef converter, UInt32 *packets, AudioBufferList *data,
                       AudioStreamPacketDescription **descriptions, void *opaque) {
  (void)converter;
  struct feed *feed = opaque;
  if (feed->given) {
    *packets = 0;
    return GIVEN;
  }
  feed->given = 1;
  data->mNumberBuffers = 1;
  data->mBuffers[0] = (AudioBuffer){(UInt32)feed->channels, feed->size, (void *)feed->data};
  feed->description = (AudioStreamPacketDescription){0, 0, feed->size};
  if (descriptions) *descriptions = &feed->description;
  *packets = 1;
  return noErr;
}

int yapfast_aac_decode(struct yapfast_aac *aac, const uint8_t *packet, size_t size,
                       const float **samples) {
  if (size == 0 || size > UINT32_MAX) return -1;
  struct feed feed = {packet, (UInt32)size, aac->channels, 0, {0}};
  UInt32 frames = MOST_FRAMES;
  AudioBufferList list = {
      .mNumberBuffers = 1,
      .mBuffers = {{(UInt32)aac->channels,
                    (UInt32)(MOST_FRAMES * (size_t)aac->channels * sizeof(float)), aac->samples}},
  };
  OSStatus status = AudioConverterFillComplexBuffer(aac->converter, supply, &feed, &frames, &list, NULL);
  if (status != noErr && status != GIVEN) return -1;
  *samples = aac->samples;
  return (int)frames;
}

void yapfast_aac_reset(struct yapfast_aac *aac) {
  AudioConverterReset(aac->converter);
}
