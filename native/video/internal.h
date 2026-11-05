// FFmpeg and libplacebo structures remain private to the interoperability shim
#pragma once

#include "flowa_video.h"

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_vulkan.h>
#include <libavutil/pixdesc.h>
#include <libplacebo/renderer.h>
#include <libplacebo/vulkan.h>
#include <libswresample/swresample.h>
#include <stdbool.h>

struct gpu_flight {
    struct gpu_flight *next;
    VkSemaphore semaphore;
    uint64_t value;
    AVFrame *source;
    AVFrame *converted;
};

struct flowa_gpu {
    pl_log log;
    pl_vulkan vk;
    pl_renderer renderer;
    AVBufferRef *av_device;
    AVBufferRef *upload_pool;
    enum AVPixelFormat upload_format;
    int upload_width;
    int upload_height;
    const AVHWDeviceContext *upload_device;
    bool video_queue;
    char render_node[128];
    pl_tex software_planes[4];
    pl_tex targets[FLOWA_VIDEO_SLOTS];
    VkImage images[FLOWA_VIDEO_SLOTS];
    struct gpu_flight *flights;
    int flight_count;
    void (*logger)(int, const char *);
};

struct flowa_media {
    AVFormatContext *format;
    AVCodecContext *audio;
    SwrContext *resampler;
    int video_stream;
    int audio_stream;
};

// An immutable snapshot avoids sharing the demuxer's mutable stream context
struct flowa_stream {
    AVFormatContext *container;
    AVStream *video;
};

struct flowa_decoder {
    AVCodecContext *codec;
    enum AVPixelFormat format;
};

int video_error(char *error, size_t size, const char *format, ...);
int video_av_error(char *error, size_t size, const char *message, int code);
void video_gpu_clear_targets(struct flowa_gpu *gpu);
void video_gpu_collect(struct flowa_gpu *gpu, bool finish);
AVBufferRef *video_gpu_frame_pool(struct flowa_gpu *gpu, const AVFrame *source);
