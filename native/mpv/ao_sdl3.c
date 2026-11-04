/*
 * SDL3 audio output for Flowa's embedded mpv
 * SPDX-License-Identifier: LGPL-2.1-or-later
 */

#include <SDL3/SDL.h>

#include "audio/format.h"
#include "common/common.h"
#include "common/msg.h"
#include "mpv_talloc.h"
#include "osdep/timer.h"
#include "ao.h"
#include "internal.h"

#define BUFFER_FRAMES 4096

struct priv {
    SDL_AudioStream *stream;
    void *buffer;
    double device_delay;
    bool initialized;
};

// SDL's documented channel order differs from WAVE for several channel counts
static const struct mp_chmap channel_maps[] = {
    MP_CHMAP_INIT_MONO,
    MP_CHMAP_INIT_STEREO,
    MP_CHMAP3(FL, FR, LFE),
    MP_CHMAP4(FL, FR, BL, BR),
    MP_CHMAP5(FL, FR, LFE, BL, BR),
    MP_CHMAP6(FL, FR, FC, LFE, SL, SR),
    MP_CHMAP7(FL, FR, FC, LFE, BC, SL, SR),
    MP_CHMAP8(FL, FR, FC, LFE, BL, BR, SL, SR),
};

static const struct {
    int mpv;
    SDL_AudioFormat sdl;
} formats[] = {
    {AF_FORMAT_U8, SDL_AUDIO_U8},
    {AF_FORMAT_S16, SDL_AUDIO_S16},
    {AF_FORMAT_S32, SDL_AUDIO_S32},
    {AF_FORMAT_FLOAT, SDL_AUDIO_F32},
};

static void audio_callback(void *userdata, SDL_AudioStream *stream,
                           int additional_bytes, int total_bytes)
{
    struct ao *ao = userdata;
    struct priv *p = ao->priv;

    // SDL holds the stream lock here and may request more than one device period
    while (additional_bytes > 0) {
        int frames = MPMIN(BUFFER_FRAMES,
                           (additional_bytes + ao->sstride - 1) / ao->sstride);
        int bytes = frames * ao->sstride;
        int queued = MPMAX(SDL_GetAudioStreamQueued(stream), 0);
        double delay = p->device_delay + (queued + bytes) / (double)ao->bps;
        void *data[] = {p->buffer};

        ao_read_data(ao, data, frames, mp_time_ns() + MP_TIME_S_TO_NS(delay),
                     NULL, true, true);

        if (!SDL_PutAudioStreamData(stream, p->buffer, bytes)) {
            MP_ERR(ao, "Cannot queue SDL3 audio: %s\n", SDL_GetError());
            return;
        }

        additional_bytes -= bytes;
    }
}

static void uninit(struct ao *ao)
{
    struct priv *p = ao->priv;

    // Destroying the stream waits for callbacks and closes its associated device
    SDL_DestroyAudioStream(p->stream);
    p->stream = NULL;

    if (p->initialized) {
        SDL_QuitSubSystem(SDL_INIT_AUDIO);
        p->initialized = false;
    }
}

static int init(struct ao *ao)
{
    struct priv *p = ao->priv;

    if (!SDL_InitSubSystem(SDL_INIT_AUDIO)) {
        MP_ERR(ao, "Cannot initialize SDL3 audio: %s\n", SDL_GetError());
        return -1;
    }

    p->initialized = true;
    struct mp_chmap_sel selection = {0};

    for (int i = 0; i < MP_ARRAY_SIZE(channel_maps); i++)
        mp_chmap_sel_add_map(&selection, &channel_maps[i]);

    if (!ao_chmap_sel_adjust(ao, &selection, &ao->channels)) {
        uninit(ao);
        return -1;
    }

    ao->format = af_fmt_from_planar(ao->format);
    SDL_AudioFormat format = SDL_AUDIO_F32;
    int mpv_format = AF_FORMAT_FLOAT;

    for (int i = 0; i < MP_ARRAY_SIZE(formats); i++) {
        if (ao->format == formats[i].mpv) {
            format = formats[i].sdl;
            mpv_format = formats[i].mpv;
            break;
        }
    }

    ao->format = mpv_format;
    int frame_bytes = af_fmt_to_bytes(ao->format) * ao->channels.num;
    p->buffer = talloc_size(p, BUFFER_FRAMES * frame_bytes);

    SDL_AudioSpec requested = {
        .format = format,
        .channels = ao->channels.num,
        .freq = ao->samplerate,
    };

    // SDL converts from this exact input format to the host device's format
    p->stream = SDL_OpenAudioDeviceStream(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
                                          &requested, audio_callback, ao);

    if (!p->stream) {
        MP_ERR(ao, "Cannot open SDL3 audio: %s\n", SDL_GetError());
        uninit(ao);
        return -1;
    }

    SDL_AudioSpec device;
    int period_frames;

    if (!SDL_GetAudioDeviceFormat(SDL_GetAudioStreamDevice(p->stream),
                                   &device, &period_frames)) {
        uninit(ao);
        return -1;
    }

    p->device_delay = 2.0 * period_frames / device.freq;
    ao->device_buffer = MPMAX(BUFFER_FRAMES, p->device_delay * ao->samplerate);
    MP_VERBOSE(ao, "SDL3 audio driver: %s\n", SDL_GetCurrentAudioDriver());
    return 1;
}

static void reset(struct ao *ao)
{
    struct priv *p = ao->priv;

    SDL_PauseAudioStreamDevice(p->stream);
    SDL_ClearAudioStream(p->stream);
}

static void start(struct ao *ao)
{
    struct priv *p = ao->priv;

    if (!SDL_ResumeAudioStreamDevice(p->stream))
        MP_ERR(ao, "Cannot start SDL3 audio: %s\n", SDL_GetError());
}

const struct ao_driver audio_out_sdl3 = {
    .description = "SDL3 Audio",
    .name = "sdl3",
    .init = init,
    .uninit = uninit,
    .reset = reset,
    .start = start,
    .priv_size = sizeof(struct priv),
};
