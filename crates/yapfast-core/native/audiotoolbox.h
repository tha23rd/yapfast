/* AAC decoding through AudioToolbox, for macOS; see audiotoolbox.c. */
#ifndef YAPFAST_AUDIOTOOLBOX_H
#define YAPFAST_AUDIOTOOLBOX_H

#include <stddef.h>
#include <stdint.h>

struct yapfast_aac;

/* A decoder of the AAC an MP4 file's AudioSpecificConfig, `config`,
 * describes, with the rate and channels it decodes to; NULL when
 * AudioToolbox can't decode it. */
struct yapfast_aac *yapfast_aac_new(const uint8_t *config, size_t config_size, int *rate,
                                    int *channels);
void yapfast_aac_free(struct yapfast_aac *aac);
/* Decodes one packet: the number of frames of interleaved floats at
 * `*samples`, valid until the next call, 0 for none yet, or a negative
 * number when the packet couldn't be decoded. */
int yapfast_aac_decode(struct yapfast_aac *aac, const uint8_t *packet, size_t size,
                       const float **samples);
/* Forgets what came before, as after seeking. */
void yapfast_aac_reset(struct yapfast_aac *aac);

#endif
