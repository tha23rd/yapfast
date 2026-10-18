/* H264 decoding through VideoToolbox, for macOS; see videotoolbox.c. */
#ifndef YAPFAST_VIDEOTOOLBOX_H
#define YAPFAST_VIDEOTOOLBOX_H

#include <stddef.h>
#include <stdint.h>

/* A time a picture has none of, as FFmpeg's AV_NOPTS_VALUE. */
#define YAPFAST_VT_NO_TIME INT64_MIN

struct yapfast_vt;

/* A decoded picture as I420 planes, valid until the next call on its decoder. */
struct yapfast_vt_picture {
  int width;
  int height;
  const uint8_t *data[3];
  int stride[3];
  /* 1 for BT.709, 0 for another matrix, -1 when the stream doesn't say. */
  int bt709;
  int full_range;
  int64_t time;
};

/* A decoder of H264. With `record`, an MP4 file's avcC box, each access unit
 * is its NAL units after their lengths, as MP4 keeps them; without, or with
 * parameter sets in Annex B form, each is NAL units after start codes, with
 * parameter sets coming in the stream. `delay` pictures are held back, so they
 * come out in the order they show, however B-frames reorder them. NULL when
 * the record's parameter sets can't be decoded. */
struct yapfast_vt *yapfast_vt_new(const uint8_t *record, size_t record_size, int delay);
void yapfast_vt_free(struct yapfast_vt *vt);
/* Decodes an access unit that shows at `time`. 0, or a negative number when
 * it couldn't be decoded; later ones may be. */
int yapfast_vt_send(struct yapfast_vt *vt, const uint8_t *data, size_t size, int64_t time);
/* Takes the next picture in the order they show: 1 with one, 0 with none yet. */
int yapfast_vt_receive(struct yapfast_vt *vt, struct yapfast_vt_picture *picture);
/* No more access units come: the pictures held back come out. */
void yapfast_vt_drain(struct yapfast_vt *vt);
/* Drops the pictures held back, as after seeking. */
void yapfast_vt_flush(struct yapfast_vt *vt);

#endif
