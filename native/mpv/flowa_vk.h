// Private Vulkan embedding ABI shared with Flowa's Rust bridge
#pragma once

#include <vulkan/vulkan.h>
#include "mpv/client.h"

#define FLOWA_VK_API "flowa-vulkan"
#define FLOWA_VK_INIT 1000
#define FLOWA_VK_TARGET 1001
#define FLOWA_VK_SLOTS 3

struct flowa_vk_init {
    VkInstance instance;
    VkPhysicalDevice physical_device;
    VkDevice device;
    PFN_vkGetInstanceProcAddr get_proc_addr;
    uint32_t queue_family;
    const char *const *extensions;
    int num_extensions;
    const VkPhysicalDeviceFeatures2 *features;
};

struct flowa_vk_target {
    VkImage image;
    uint32_t width;
    uint32_t height;
    uint32_t slot;
    VkSemaphore semaphore;
    uint64_t wait_value;
    uint64_t signal_value;
};

MPV_EXPORT unsigned flowa_mpv_vulkan_abi_version(void);
