// Libplacebo Vulkan interoperability retained by Rust-owned media and playback
#pragma once

#include <stddef.h>
#include <stdint.h>
#include <vulkan/vulkan.h>

#define FLOWA_VIDEO_ABI 4
#define FLOWA_VIDEO_SLOTS 3

struct flowa_gpu;
struct AVFrame;
struct AVStream;
struct AVBufferRef;

struct flowa_gpu_init {
    VkInstance instance;
    VkPhysicalDevice physical_device;
    PFN_vkGetInstanceProcAddr get_proc_addr;
    const char *const *extensions;
    int num_extensions;
    const VkPhysicalDeviceFeatures2 *features;
    void (*log)(int level, const char *message);
};

struct flowa_gpu_info {
    VkDevice device;
    uint32_t graphics_family;
    const char *const *extensions;
    int num_extensions;
    const VkPhysicalDeviceFeatures2 *features;
};

struct flowa_video_target {
    VkImage image;
    VkSemaphore semaphore;
    uint64_t wait_value;
    uint64_t signal_value;
    uint32_t width;
    uint32_t height;
    uint32_t slot;
};

uint32_t flowa_video_abi_version(void);
struct flowa_gpu *flowa_gpu_create(
    const struct flowa_gpu_init *init,
    char *error,
    size_t error_size
);
void flowa_gpu_destroy(struct flowa_gpu *gpu);
void flowa_gpu_get_info(struct flowa_gpu *gpu, struct flowa_gpu_info *info);
void flowa_gpu_retire_targets(struct flowa_gpu *gpu);
void flowa_gpu_reset_frames(struct flowa_gpu *gpu);

// Transfers one FFmpeg device reference to the Rust codec owner
struct AVBufferRef *flowa_gpu_create_decoder_device(struct flowa_gpu *gpu, int backend);
int flowa_gpu_render(
    struct flowa_gpu *gpu,
    const struct AVFrame *frame,
    const struct AVStream *stream,
    const struct flowa_video_target *target,
    char *error,
    size_t error_size
);
