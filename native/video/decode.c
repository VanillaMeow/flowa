// FFmpeg hardware-context construction and individual codec operations
#include "internal.h"

#include <errno.h>
#include <libavutil/display.h>
#include <math.h>
#include <stdlib.h>

static enum AVPixelFormat hardware_format(
    AVCodecContext *context,
    const enum AVPixelFormat *formats
) {
    const struct flowa_decoder *decoder = context->opaque;

    for (
        const enum AVPixelFormat *format = formats;
        *format != AV_PIX_FMT_NONE;
        format++
    ) {
        if (*format == decoder->format)
            return *format;
    }

    // Returning NONE prevents FFmpeg from silently choosing a software format
    return AV_PIX_FMT_NONE;
}

static AVBufferRef *hardware_device(struct flowa_gpu *gpu, enum AVHWDeviceType type) {
    AVBufferRef *device = NULL;

    if (type == AV_HWDEVICE_TYPE_VULKAN)
        return gpu->video_queue && gpu->av_device ? av_buffer_ref(gpu->av_device)
                                                  : NULL;

    if (type == AV_HWDEVICE_TYPE_CUDA && gpu->av_device)
        av_hwdevice_ctx_create_derived(&device, type, gpu->av_device, 0);

    if (type == AV_HWDEVICE_TYPE_VAAPI && gpu->render_node[0])
        av_hwdevice_ctx_create(&device, type, gpu->render_node, NULL, 0);

    return device;
}

static const AVCodec *hardware_codec(
    enum AVCodecID id,
    enum AVHWDeviceType type,
    enum AVPixelFormat *format
) {
    void *iterator = NULL;
    const AVCodec *codec;

    // The default AV1 decoder can be libdav1d, which has no hardware configurations
    while ((codec = av_codec_iterate(&iterator))) {
        if (codec->id != id || !av_codec_is_decoder(codec))
            continue;

        const AVCodecHWConfig *configuration;
        for (
            int index = 0;
            (configuration = avcodec_get_hw_config(codec, index));
            index++
        ) {
            if (
                configuration->device_type == type &&
                configuration->methods & AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX
            ) {
                *format = configuration->pix_fmt;
                return codec;
            }
        }
    }
    return NULL;
}

struct flowa_decoder *flowa_decoder_create(
    struct flowa_gpu *gpu,
    const struct flowa_stream *stream,
    int backend,
    char *error,
    size_t size
) {
    const enum AVHWDeviceType types[] = {
        AV_HWDEVICE_TYPE_CUDA,
        AV_HWDEVICE_TYPE_VULKAN,
        AV_HWDEVICE_TYPE_VAAPI,
    };

    if (backend < 0 || backend >= 3) {
        video_error(error, size, "Invalid hardware decoder backend");
        return NULL;
    }

    struct flowa_decoder *decoder = calloc(1, sizeof(*decoder));
    if (!decoder) {
        video_error(error, size, "Cannot allocate the hardware decoder");
        return NULL;
    }

    const AVCodec *codec = hardware_codec(
        stream->video->codecpar->codec_id,
        types[backend],
        &decoder->format
    );
    AVBufferRef *device = codec ? hardware_device(gpu, types[backend]) : NULL;
    if (!device) {
        video_error(
            error,
            size,
            "The hardware backend does not support this codec or GPU"
        );
        flowa_decoder_free(decoder);
        return NULL;
    }

    decoder->codec = avcodec_alloc_context3(codec);
    if (!decoder->codec) {
        av_buffer_unref(&device);
        video_error(error, size, "Cannot allocate the FFmpeg codec context");
        flowa_decoder_free(decoder);
        return NULL;
    }

    decoder->codec->hw_device_ctx = device;
    int result = avcodec_parameters_to_context(decoder->codec, stream->video->codecpar);
    if (result < 0)
        goto error;

    decoder->codec->pkt_timebase = stream->video->time_base;
    decoder->codec->get_format = hardware_format;
    decoder->codec->opaque = decoder;

    // Rust serializes video submissions with libplacebo and wgpu on the UI thread
    decoder->codec->thread_count = 1;
    decoder->codec->thread_type = 0;
    result = avcodec_open2(decoder->codec, codec, NULL);
    if (result < 0)
        goto error;

    return decoder;

error:
    video_av_error(error, size, "Cannot initialize the hardware decoder", result);
    flowa_decoder_free(decoder);
    return NULL;
}

void flowa_decoder_free(struct flowa_decoder *decoder) {
    if (!decoder)
        return;

    avcodec_free_context(&decoder->codec);
    free(decoder);
}

void flowa_decoder_flush(struct flowa_decoder *decoder) {
    avcodec_flush_buffers(decoder->codec);
}

int flowa_decoder_send(
    struct flowa_decoder *decoder,
    const struct flowa_packet *packet,
    char *error,
    size_t size
) {
    int result = avcodec_send_packet(decoder->codec, (const void *)packet);
    if (result == AVERROR(EAGAIN))
        return 1;
    if (result == AVERROR_EOF)
        return 2;
    return result < 0 ? video_av_error(error, size, "Cannot send video packet", result)
                      : 0;
}

int flowa_decoder_receive(
    struct flowa_decoder *decoder,
    struct flowa_frame **output,
    char *error,
    size_t size
) {
    AVFrame *frame = av_frame_alloc();
    if (!frame)
        return video_error(error, size, "Cannot allocate a video frame");

    int result = avcodec_receive_frame(decoder->codec, frame);
    if (result < 0) {
        av_frame_free(&frame);
        if (result == AVERROR(EAGAIN))
            return 0;
        return result == AVERROR_EOF
                   ? 2
                   : video_av_error(error, size, "Hardware decoding failed", result);
    }

    if (!frame->hw_frames_ctx || frame->format != decoder->format) {
        av_frame_free(&frame);
        return video_error(
            error,
            size,
            "The decoder returned a software frame; refusing CPU video decoding"
        );
    }

    *output = (void *)frame;
    return 1;
}

void flowa_frame_free(struct flowa_frame *frame) {
    AVFrame *raw = (void *)frame;
    av_frame_free(&raw);
}

void flowa_frame_get_info(
    const struct flowa_frame *frame,
    struct flowa_frame_info *info
) {
    const AVFrame *raw = (const void *)frame;
    *info = (struct flowa_frame_info){
        .timestamp = raw->best_effort_timestamp,
        .width = raw->width,
        .height = raw->height,
        .aspect_num = raw->sample_aspect_ratio.num,
        .aspect_den = raw->sample_aspect_ratio.den,
        .rotation = NAN,
    };

    const AVFrameSideData *matrix =
        av_frame_get_side_data(raw, AV_FRAME_DATA_DISPLAYMATRIX);
    if (matrix && matrix->size >= 9 * sizeof(int32_t))
        info->rotation = av_display_rotation_get((const int32_t *)matrix->data);
}
