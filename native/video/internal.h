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

#define FLOWA_MAX_DEVICE_EXTENSIONS 128

// Feature structures enabled on a device that flowa creates, kept alive with it
struct device_features {
    VkPhysicalDeviceFeatures2 core;
    VkPhysicalDeviceVulkan11Features v11;
    VkPhysicalDeviceVulkan12Features v12;
    VkPhysicalDeviceVulkan13Features v13;
    VkPhysicalDeviceVideoMaintenance1FeaturesKHR video_maintenance1;
#ifdef VK_KHR_video_maintenance2
    VkPhysicalDeviceVideoMaintenance2FeaturesKHR video_maintenance2;
#endif
#ifdef VK_KHR_video_decode_vp9
    VkPhysicalDeviceVideoDecodeVP9FeaturesKHR vp9;
#endif
#ifdef VK_EXT_swapchain_maintenance1
    VkPhysicalDeviceSwapchainMaintenance1FeaturesEXT swapchain_maintenance1;
#endif
#ifdef VK_EXT_host_image_copy
    VkPhysicalDeviceHostImageCopyFeaturesEXT host_image_copy;
#endif
    VkPhysicalDeviceRobustness2FeaturesEXT robustness2;
};

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

    // Extensions and features enabled on the device, whichever path created it
    const char *extensions[FLOWA_MAX_DEVICE_EXTENSIONS];
    int num_extensions;
    const VkPhysicalDeviceFeatures2 *enabled_features;

    // Set when flowa created the device itself rather than through libplacebo
    VkDevice device;
    PFN_vkDestroyDevice destroy_device;
    struct device_features features;
};

pl_vulkan video_gpu_create_vulkan(
    struct flowa_gpu *gpu,
    const struct flowa_gpu_init *init
);
void video_gpu_destroy_vulkan(struct flowa_gpu *gpu);
bool video_gpu_has_extension(const struct flowa_gpu *gpu, const char *name);

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
