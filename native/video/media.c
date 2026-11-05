// Individual FFmpeg I/O and resampling operations called by the Rust worker
#include "internal.h"

#include <errno.h>
#include <libavutil/display.h>
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

int video_error(char *error, size_t size, const char *format, ...) {
    va_list arguments;
    va_start(arguments, format);
    vsnprintf(error, size, format, arguments);
    va_end(arguments);
    return -1;
}

int video_av_error(char *error, size_t size, const char *message, int code) {
    char detail[AV_ERROR_MAX_STRING_SIZE];
    av_strerror(code, detail, sizeof(detail));
    return video_error(error, size, "%s: %s", message, detail);
}

static int open_audio(struct flowa_media *media) {
    if (media->audio_stream < 0)
        return 0;

    AVStream *stream = media->format->streams[media->audio_stream];
    const AVCodec *codec = avcodec_find_decoder(stream->codecpar->codec_id);
    media->audio = codec ? avcodec_alloc_context3(codec) : NULL;
    if (!media->audio)
        return AVERROR_DECODER_NOT_FOUND;

    int result = avcodec_parameters_to_context(media->audio, stream->codecpar);
    if (result < 0)
        return result;

    media->audio->pkt_timebase = stream->time_base;
    result = avcodec_open2(media->audio, codec, NULL);
    if (result < 0)
        return result;

    AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
    result = swr_alloc_set_opts2(
        &media->resampler,
        &stereo,
        AV_SAMPLE_FMT_FLT,
        48000,
        &media->audio->ch_layout,
        media->audio->sample_fmt,
        media->audio->sample_rate,
        0,
        NULL
    );
    return result < 0 ? result : swr_init(media->resampler);
}

struct flowa_media *flowa_media_open(
    const char *path,
    int (*interrupt)(void *),
    void *userdata,
    struct flowa_stream_info *info,
    char *error,
    size_t size
) {
    struct flowa_media *media = calloc(1, sizeof(*media));
    if (!media)
        goto allocation_error;

    media->format = avformat_alloc_context();
    if (!media->format)
        goto allocation_error;

    media->format->interrupt_callback = (AVIOInterruptCB){interrupt, userdata};
    int result = avformat_open_input(&media->format, path, NULL, NULL);
    if (result < 0)
        goto error;

    result = avformat_find_stream_info(media->format, NULL);
    if (result < 0)
        goto error;

    media->video_stream =
        av_find_best_stream(media->format, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    result = media->video_stream;
    if (result < 0)
        goto error;

    media->audio_stream =
        av_find_best_stream(media->format, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0);
    result = open_audio(media);
    if (result < 0)
        goto error;

    AVStream *video = media->format->streams[media->video_stream];
    AVRational rate = av_guess_frame_rate(media->format, video, NULL);
    *info = (struct flowa_stream_info){
        .origin = media->format->start_time == AV_NOPTS_VALUE
                      ? 0
                      : media->format->start_time / (double)AV_TIME_BASE,
        .duration = media->format->duration == AV_NOPTS_VALUE
                        ? 0
                        : media->format->duration / (double)AV_TIME_BASE,
        .video_time_base = av_q2d(video->time_base),
        .audio_time_base =
            media->audio_stream >= 0
                ? av_q2d(media->format->streams[media->audio_stream]->time_base)
                : 0,
        .frame_duration =
            rate.num > 0 && rate.den > 0 ? av_q2d(av_inv_q(rate)) : 1.0 / 30.0,
        .has_audio = media->audio_stream >= 0,
    };
    if (info->duration <= 0 && video->duration != AV_NOPTS_VALUE)
        info->duration = video->duration * info->video_time_base;

    const AVPacketSideData *matrix = av_packet_side_data_get(
        video->codecpar->coded_side_data,
        video->codecpar->nb_coded_side_data,
        AV_PKT_DATA_DISPLAYMATRIX
    );
    if (matrix && matrix->size >= 9 * sizeof(int32_t))
        info->rotation = av_display_rotation_get((const int32_t *)matrix->data);
    snprintf(
        info->codec,
        sizeof(info->codec),
        "%s",
        avcodec_get_name(video->codecpar->codec_id)
    );
    return media;

allocation_error:
    video_error(error, size, "Cannot allocate the FFmpeg media context");
    flowa_media_close(media);
    return NULL;

error:
    video_av_error(error, size, "Cannot open media", result);
    flowa_media_close(media);
    return NULL;
}

void flowa_media_close(struct flowa_media *media) {
    if (!media)
        return;

    avcodec_free_context(&media->audio);
    swr_free(&media->resampler);
    avformat_close_input(&media->format);
    free(media);
}

struct flowa_stream *flowa_media_copy_stream(struct flowa_media *media) {
    struct flowa_stream *stream = calloc(1, sizeof(*stream));
    if (!stream)
        return NULL;

    stream->container = avformat_alloc_context();
    if (!stream->container)
        goto error;

    stream->video = avformat_new_stream(stream->container, NULL);
    if (!stream->video)
        goto error;

    const AVStream *source = media->format->streams[media->video_stream];
    if (avcodec_parameters_copy(stream->video->codecpar, source->codecpar) < 0)
        goto error;

    stream->video->time_base = source->time_base;
    stream->video->sample_aspect_ratio = source->sample_aspect_ratio;
    return stream;

error:
    flowa_stream_free(stream);
    return NULL;
}

void flowa_stream_free(struct flowa_stream *stream) {
    if (!stream)
        return;

    avformat_free_context(stream->container);
    free(stream);
}

int flowa_media_read(
    struct flowa_media *media,
    struct flowa_packet **output,
    int *kind,
    int *bytes,
    char *error,
    size_t size
) {
    AVPacket *packet = av_packet_alloc();
    if (!packet)
        return video_error(error, size, "Cannot allocate a demux packet");

    int result = av_read_frame(media->format, packet);
    if (result < 0) {
        av_packet_free(&packet);
        return result == AVERROR_EOF
                   ? 2
                   : video_av_error(error, size, "Cannot read media", result);
    }

    *kind = packet->stream_index == media->video_stream ? 1
            : packet->stream_index == media->audio_stream ? 2
                                                          : 0;
    *bytes = packet->size;
    *output = (void *)packet;
    return 1;
}

int flowa_media_seek(
    struct flowa_media *media,
    int64_t timestamp,
    char *error,
    size_t size
) {
    int result = avformat_seek_file(
        media->format,
        -1,
        INT64_MIN,
        timestamp,
        timestamp,
        AVSEEK_FLAG_BACKWARD
    );
    if (result < 0)
        return video_av_error(error, size, "Cannot seek", result);

    if (media->audio) {
        avcodec_flush_buffers(media->audio);
        swr_close(media->resampler);
        result = swr_init(media->resampler);
        if (result < 0)
            return video_av_error(
                error,
                size,
                "Cannot reset the audio resampler",
                result
            );
    }
    return 0;
}

struct flowa_packet *flowa_packet_clone(const struct flowa_packet *packet) {
    return (void *)av_packet_clone((const void *)packet);
}

void flowa_packet_free(struct flowa_packet *packet) {
    AVPacket *raw = (void *)packet;
    av_packet_free(&raw);
}

int flowa_media_audio_send(
    struct flowa_media *media,
    const struct flowa_packet *packet,
    char *error,
    size_t size
) {
    int result = avcodec_send_packet(media->audio, (const void *)packet);
    if (result == AVERROR(EAGAIN))
        return 1;
    if (result == AVERROR_EOF)
        return 2;
    return result < 0 ? video_av_error(error, size, "Cannot send audio packet", result)
                      : 0;
}

int flowa_media_audio_receive(
    struct flowa_media *media,
    struct flowa_audio_block *block,
    char *error,
    size_t size
) {
    AVFrame *frame = av_frame_alloc();
    if (!frame)
        return video_error(error, size, "Cannot allocate an audio frame");

    int result = avcodec_receive_frame(media->audio, frame);
    if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) {
        av_frame_free(&frame);
        return result == AVERROR_EOF ? 2 : 0;
    }
    if (result < 0)
        goto error;

    *block = (struct flowa_audio_block){
        .timestamp = frame->best_effort_timestamp,
        .resample_delay = swr_get_delay(media->resampler, 48000) / 48000.0,
    };
    int capacity = swr_get_out_samples(media->resampler, frame->nb_samples);
    if (capacity <= 0 || capacity > 480000) {
        result = AVERROR_INVALIDDATA;
        goto error;
    }

    block->samples = av_malloc_array(capacity, 2 * sizeof(float));
    if (!block->samples) {
        result = AVERROR(ENOMEM);
        goto error;
    }

    uint8_t *planes[] = {(uint8_t *)block->samples};
    result = swr_convert(
        media->resampler,
        planes,
        capacity,
        (const uint8_t **)frame->extended_data,
        frame->nb_samples
    );
    if (result < 0) {
        av_freep(&block->samples);
        goto error;
    }

    block->frames = result;
    av_frame_free(&frame);
    return 1;

error:
    av_frame_free(&frame);
    return video_av_error(error, size, "Cannot decode audio", result);
}

void flowa_audio_block_free(float *samples) {
    av_free(samples);
}
