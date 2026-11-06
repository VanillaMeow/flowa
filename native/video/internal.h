// GPU structures shared by the native Vulkan helpers
#pragma once

#include "flowa_video.h"

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_vulkan.h>
#include <libavutil/pixdesc.h>
#include <libplacebo/renderer.h>
#include <libplacebo/vulkan.h>
#include <stdbool.h>

struct gpu_flight {
    struct gpu_flight *next;
    VkSemaphore semaphore;
    uint64_t value;
    AVFrame *source;
    AVFrame *converted;
    VkCommandBuffer transfer;
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
    struct video_transfer *transfer;
    int flight_count;
    void (*logger)(int, const char *);
};

void video_gpu_clear_targets(struct flowa_gpu *gpu);
void video_gpu_collect(struct flowa_gpu *gpu, bool finish);
AVBufferRef *video_gpu_frame_pool(struct flowa_gpu *gpu, const AVFrame *source);
AVFrame *video_gpu_transfer_vulkan(
    struct flowa_gpu *gpu,
    const AVFrame *source,
    VkCommandBuffer *command
);
void video_gpu_free_transfer(
    struct flowa_gpu *gpu,
    const AVFrame *frame,
    VkCommandBuffer command
);
void video_gpu_destroy_transfer(struct flowa_gpu *gpu);
