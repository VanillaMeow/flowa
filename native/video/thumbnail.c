// Bounded CPU poster extraction independent of the playback device and audio
#include "internal.h"

#include <errno.h>
#include <libavutil/display.h>
#include <libswscale/swscale.h>
#include <math.h>

static int thumbnail_frame(
    AVFormatContext *format,
    int index,
    struct flowa_decoder *decoder,
    AVFrame *frame
) {
    AVPacket *packet = av_packet_alloc();
    if (!packet)
        return AVERROR(ENOMEM);

    int result = 0;
    int packets = 0;
    int64_t bytes = 0;
    bool draining = false;

    while (packets < 512 && bytes < 32 * 1024 * 1024) {
        if (format->interrupt_callback.callback(format->interrupt_callback.opaque)) {
            result = AVERROR_EXIT;
            break;
        }

        result = avcodec_receive_frame(decoder->codec, frame);
        if (result != AVERROR(EAGAIN))
            break;
        if (draining) {
            result = AVERROR_EOF;
            break;
        }

        result = av_read_frame(format, packet);
        if (result == AVERROR_EOF) {
            draining = true;
            result = avcodec_send_packet(decoder->codec, NULL);
            if (result < 0)
                break;
            continue;
        }
        if (result < 0)
            break;

        packets++;
        bytes += packet->size;
        if (packet->stream_index == index)
            result = avcodec_send_packet(decoder->codec, packet);
        av_packet_unref(packet);
        if (result < 0)
            break;
    }

    av_packet_free(&packet);
    return frame->width > 0 ? 0 : result < 0 ? result : AVERROR_INVALIDDATA;
}

static int thumbnail_colorspace(enum AVColorSpace space) {
    switch (space) {
        case AVCOL_SPC_BT709:
            return SWS_CS_ITU709;
        case AVCOL_SPC_BT2020_NCL:
        case AVCOL_SPC_BT2020_CL:
            return SWS_CS_BT2020;
        case AVCOL_SPC_FCC:
            return SWS_CS_FCC;
        case AVCOL_SPC_SMPTE240M:
            return SWS_CS_SMPTE240M;
        default:
            return SWS_CS_DEFAULT;
    }
}

static int scale_thumbnail(
    const AVFrame *frame,
    AVRational aspect,
    uint8_t *pixels,
    int max_width,
    int max_height,
    int *width,
    int *height
) {
    if (frame->width <= 0 || frame->height <= 0)
        return AVERROR_INVALIDDATA;

    double ratio = aspect.num > 0 && aspect.den > 0 ? av_q2d(aspect) : 1.0;
    double display_width = frame->width * ratio;
    double scale = fmin(
        1.0,
        fmin(max_width / display_width, (double)max_height / frame->height)
    );
    *width = fmax(1.0, fmin(max_width, round(display_width * scale)));
    *height = fmax(1.0, fmin(max_height, round(frame->height * scale)));

    struct SwsContext *scaler = sws_getContext(
        frame->width,
        frame->height,
        frame->format,
        *width,
        *height,
        AV_PIX_FMT_RGBA,
        SWS_BILINEAR,
        NULL,
        NULL,
        NULL
    );
    if (!scaler)
        return AVERROR(ENOMEM);

    const int *coefficients =
        sws_getCoefficients(thumbnail_colorspace(frame->colorspace));
    sws_setColorspaceDetails(
        scaler,
        coefficients,
        frame->color_range == AVCOL_RANGE_JPEG,
        coefficients,
        1,
        0,
        65536,
        65536
    );
    uint8_t *planes[] = {pixels, NULL, NULL, NULL};
    int strides[] = {*width * 4, 0, 0, 0};
    int result = sws_scale(
        scaler,
        (const uint8_t *const *)frame->data,
        frame->linesize,
        0,
        frame->height,
        planes,
        strides
    );
    sws_freeContext(scaler);
    return result == *height ? 0 : AVERROR_INVALIDDATA;
}

int flowa_video_thumbnail(
    const char *path,
    int (*interrupt)(void *),
    void *userdata,
    uint8_t *pixels,
    int max_width,
    int max_height,
    int *width,
    int *height,
    double *rotation,
    char *error,
    size_t size
) {
    if (
        !interrupt || max_width <= 0 || max_height <= 0 || max_width > 256 ||
        max_height > 256
    ) {
        return video_error(error, size, "Invalid thumbnail request");
    }

    AVFormatContext *format = avformat_alloc_context();
    AVFrame *frame = av_frame_alloc();
    struct flowa_decoder *decoder = NULL;
    int result = AVERROR(ENOMEM);
    if (!format || !frame)
        goto done;

    format->interrupt_callback = (AVIOInterruptCB){interrupt, userdata};
    format->probesize = 2 * 1024 * 1024;
    format->max_analyze_duration = AV_TIME_BASE;
    result = avformat_open_input(&format, path, NULL, NULL);
    if (result < 0)
        goto done;

    result = avformat_find_stream_info(format, NULL);
    if (result < 0)
        goto done;

    int index = av_find_best_stream(format, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    result = index;
    if (result < 0)
        goto done;

    AVStream *video = format->streams[index];
    struct flowa_stream stream = {.container = format, .video = video};
    decoder = flowa_decoder_create(NULL, &stream, FLOWA_DECODER_SOFTWARE, error, size);
    if (!decoder) {
        result = -1;
        goto done;
    }

    // A nearby keyframe avoids decoding a long prefix just to build a poster
    if (format->duration > 0) {
        double seconds = fmin(3.0, format->duration / (10.0 * AV_TIME_BASE));
        int64_t start = video->start_time == AV_NOPTS_VALUE ? 0 : video->start_time;
        int64_t target = start + seconds / av_q2d(video->time_base);
        av_seek_frame(format, index, target, AVSEEK_FLAG_BACKWARD);
    }

    result = thumbnail_frame(format, index, decoder, frame);
    if (result < 0)
        goto done;

    *rotation = 0;
    const AVFrameSideData *matrix = av_frame_get_side_data(
        frame,
        AV_FRAME_DATA_DISPLAYMATRIX
    );
    const AVPacketSideData *stream_matrix = av_packet_side_data_get(
        video->codecpar->coded_side_data,
        video->codecpar->nb_coded_side_data,
        AV_PKT_DATA_DISPLAYMATRIX
    );
    if (matrix && matrix->size >= 9 * sizeof(int32_t))
        *rotation = av_display_rotation_get((const int32_t *)matrix->data);
    else if (stream_matrix && stream_matrix->size >= 9 * sizeof(int32_t))
        *rotation = av_display_rotation_get((const int32_t *)stream_matrix->data);

    result = scale_thumbnail(
        frame,
        av_guess_sample_aspect_ratio(format, video, frame),
        pixels,
        max_width,
        max_height,
        width,
        height
    );

done:
    flowa_decoder_free(decoder);
    av_frame_free(&frame);
    avformat_close_input(&format);
    return result < 0 ? video_av_error(
        error,
        size,
        "Cannot extract video thumbnail",
        result
    ) : 0;
}
