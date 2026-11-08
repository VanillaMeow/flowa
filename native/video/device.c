// Shared Vulkan device creation with only the queues that flowa uses
//
// Libplacebo always creates every queue of every queue family, which costs over
// 100 ms on some NVIDIA drivers. Creating the device here and importing it keeps
// startup fast while enabling the same extensions and features
#include "internal.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_QUEUE_FAMILIES 16

static const float QUEUE_PRIORITIES[] = {1.0f, 1.0f};

// Extension feature structures wgpu may request that Vulkan 1.1-1.3 core covers
static const VkStructureType PROMOTED_FEATURES[] = {
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROTECTED_MEMORY_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DRAW_PARAMETERS_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_8BIT_STORAGE_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ATOMIC_INT64_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT16_INT8_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SCALAR_BLOCK_LAYOUT_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_MEMORY_MODEL_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGELESS_FRAMEBUFFER_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_UNIFORM_BUFFER_STANDARD_LAYOUT_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_EXTENDED_TYPES_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SEPARATE_DEPTH_STENCIL_LAYOUTS_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_QUERY_RESET_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_TERMINATE_INVOCATION_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DEMOTE_TO_HELPER_INVOCATION_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRIVATE_DATA_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_CREATION_CACHE_CONTROL_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SYNCHRONIZATION_2_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ZERO_INITIALIZE_WORKGROUP_MEMORY_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_ROBUSTNESS_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INLINE_UNIFORM_BLOCK_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXTURE_COMPRESSION_ASTC_HDR_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_INTEGER_DOT_PRODUCT_FEATURES,
    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_4_FEATURES,
};

struct queue_plan {
    VkDeviceQueueCreateInfo families[MAX_QUEUE_FAMILIES];
    uint32_t count;
    struct pl_vulkan_queue graphics;
    struct pl_vulkan_queue compute;
    struct pl_vulkan_queue transfer;
};

static void log_warning(struct flowa_gpu *gpu, const char *message) {
    if (gpu->logger)
        gpu->logger(PL_LOG_WARN, message);
}

static bool is_promoted(VkStructureType type) {
    size_t count = sizeof(PROMOTED_FEATURES) / sizeof(*PROMOTED_FEATURES);

    for (size_t index = 0; index < count; index++) {
        if (PROMOTED_FEATURES[index] == type)
            return true;
    }

    return false;
}

// Whether every structure wgpu chains can be expressed by the features enabled here
static bool covers_wgpu_features(const VkPhysicalDeviceFeatures2 *requested) {
    if (!requested)
        return true;

    for (
        const VkBaseInStructure *node = requested->pNext;
        node;
        node = node->pNext
    ) {
        bool robustness = node->sType ==
            VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_FEATURES_EXT;

        if (!is_promoted(node->sType) && !robustness)
            return false;
    }

    return true;
}

bool video_gpu_has_extension(const struct flowa_gpu *gpu, const char *name) {
    for (int index = 0; index < gpu->num_extensions; index++) {
        if (!strcmp(gpu->extensions[index], name))
            return true;
    }

    return false;
}

static void enable_extension(
    struct flowa_gpu *gpu,
    const VkExtensionProperties *available,
    uint32_t available_count,
    const char *name
) {
    bool full = gpu->num_extensions == FLOWA_MAX_DEVICE_EXTENSIONS;

    if (full || video_gpu_has_extension(gpu, name))
        return;

    for (uint32_t index = 0; index < available_count; index++) {
        if (!strcmp(available[index].extensionName, name)) {
            gpu->extensions[gpu->num_extensions++] = name;
            return;
        }
    }
}

// Enables the supported subset of wgpu, libplacebo, and FFmpeg decoding extensions
static bool enable_extensions(
    struct flowa_gpu *gpu,
    const struct flowa_gpu_init *init,
    const char **decoding,
    int decoding_count
) {
    PFN_vkEnumerateDeviceExtensionProperties enumerate = (void *)init->get_proc_addr(
        init->instance,
        "vkEnumerateDeviceExtensionProperties"
    );
    uint32_t count = 0;
    enumerate(init->physical_device, NULL, &count, NULL);

    VkExtensionProperties *available = calloc(count, sizeof(*available));
    if (!available)
        return false;

    enumerate(init->physical_device, NULL, &count, available);

    for (int index = 0; index < init->num_extensions; index++)
        enable_extension(gpu, available, count, init->extensions[index]);

    // The DRM render node identifies the GPU when opening VA-API
    enable_extension(gpu, available, count, VK_EXT_PHYSICAL_DEVICE_DRM_EXTENSION_NAME);

    for (int index = 0; index < pl_vulkan_num_recommended_extensions; index++) {
        enable_extension(
            gpu,
            available,
            count,
            pl_vulkan_recommended_extensions[index]
        );
    }

    for (int index = 0; index < decoding_count; index++)
        enable_extension(gpu, available, count, decoding[index]);

    free(available);
    return true;
}

static void append_feature(VkBaseOutStructure **tail, void *structure) {
    (*tail)->pNext = structure;
    *tail = structure;
}

// Enables every supported core and extension feature, plus wgpu's robustness request
static void enable_features(
    struct flowa_gpu *gpu,
    const struct flowa_gpu_init *init
) {
    struct device_features *features = &gpu->features;
    *features = (struct device_features){
        .core = {.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2},
        .v11 = {.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES},
        .v12 = {.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES},
        .v13 = {.sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES},
        .video_maintenance1 = {
            .sType =
                VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_MAINTENANCE_1_FEATURES_KHR,
        },
#ifdef VK_KHR_video_maintenance2
        .video_maintenance2 = {
            .sType =
                VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_MAINTENANCE_2_FEATURES_KHR,
        },
#endif
#ifdef VK_KHR_video_decode_vp9
        .vp9 = {
            .sType =
                VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_DECODE_VP9_FEATURES_KHR,
        },
#endif
#ifdef VK_EXT_swapchain_maintenance1
        .swapchain_maintenance1 = {
            .sType =
                VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SWAPCHAIN_MAINTENANCE_1_FEATURES_EXT,
        },
#endif
#ifdef VK_EXT_host_image_copy
        .host_image_copy = {
            .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_IMAGE_COPY_FEATURES_EXT,
        },
#endif
    };

    // Extension structures may only be chained when their extension is enabled
    VkBaseOutStructure *tail = (void *)&features->core;
    append_feature(&tail, &features->v11);
    append_feature(&tail, &features->v12);
    append_feature(&tail, &features->v13);

    if (video_gpu_has_extension(gpu, VK_KHR_VIDEO_MAINTENANCE_1_EXTENSION_NAME))
        append_feature(&tail, &features->video_maintenance1);
#ifdef VK_KHR_video_maintenance2
    if (video_gpu_has_extension(gpu, VK_KHR_VIDEO_MAINTENANCE_2_EXTENSION_NAME))
        append_feature(&tail, &features->video_maintenance2);
#endif
#ifdef VK_KHR_video_decode_vp9
    if (video_gpu_has_extension(gpu, VK_KHR_VIDEO_DECODE_VP9_EXTENSION_NAME))
        append_feature(&tail, &features->vp9);
#endif
#ifdef VK_EXT_swapchain_maintenance1
    if (
        video_gpu_has_extension(gpu, VK_EXT_SWAPCHAIN_MAINTENANCE_1_EXTENSION_NAME) ||
        video_gpu_has_extension(gpu, "VK_KHR_swapchain_maintenance1")
    ) {
        append_feature(&tail, &features->swapchain_maintenance1);
    }
#endif
#ifdef VK_EXT_host_image_copy
    if (video_gpu_has_extension(gpu, VK_EXT_HOST_IMAGE_COPY_EXTENSION_NAME))
        append_feature(&tail, &features->host_image_copy);
#endif

    PFN_vkGetPhysicalDeviceFeatures2 get_features = (void *)init->get_proc_addr(
        init->instance,
        "vkGetPhysicalDeviceFeatures2"
    );
    get_features(init->physical_device, &features->core);

    // Copy wgpu's choice so robust access costs nothing it did not ask for
    if (!init->features)
        return;

    for (
        const VkBaseInStructure *node = init->features->pNext;
        node;
        node = node->pNext
    ) {
        if (node->sType != VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_FEATURES_EXT)
            continue;

        memcpy(&features->robustness2, node, sizeof(features->robustness2));
        features->robustness2.pNext = NULL;
        append_feature(&tail, &features->robustness2);
    }
}

static void plan_family(struct queue_plan *plan, uint32_t family, uint32_t queues) {
    for (uint32_t index = 0; index < plan->count; index++) {
        if (plan->families[index].queueFamilyIndex == family)
            return;
    }

    if (plan->count == MAX_QUEUE_FAMILIES)
        return;

    plan->families[plan->count++] = (VkDeviceQueueCreateInfo){
        .sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
        .queueFamilyIndex = family,
        .queueCount = queues,
        .pQueuePriorities = QUEUE_PRIORITIES,
    };
}

// Picks the graphics, async compute, async transfer, and video decode families
static bool plan_queues(
    const struct flowa_gpu_init *init,
    struct queue_plan *plan
) {
    PFN_vkGetPhysicalDeviceQueueFamilyProperties get_families = (void *)
        init->get_proc_addr(init->instance, "vkGetPhysicalDeviceQueueFamilyProperties");
    uint32_t count = 0;
    get_families(init->physical_device, &count, NULL);

    VkQueueFamilyProperties families[MAX_QUEUE_FAMILIES];
    count = count < MAX_QUEUE_FAMILIES ? count : MAX_QUEUE_FAMILIES;
    get_families(init->physical_device, &count, families);

    int graphics = -1;
    int compute = -1;
    int transfer = -1;

    for (uint32_t index = 0; index < count; index++) {
        VkQueueFlags flags = families[index].queueFlags;
        bool has_graphics = flags & VK_QUEUE_GRAPHICS_BIT;
        bool has_compute = flags & VK_QUEUE_COMPUTE_BIT;
        bool has_transfer = flags & VK_QUEUE_TRANSFER_BIT;

        if (graphics < 0 && has_graphics)
            graphics = (int)index;
        if (compute < 0 && has_compute && !has_graphics)
            compute = (int)index;
        if (transfer < 0 && has_transfer && !has_graphics && !has_compute)
            transfer = (int)index;
    }

    if (graphics < 0)
        return false;

    // Playback renders on queue 0; wgpu takes queue 1 when the family has one
    uint32_t graphics_queues = families[graphics].queueCount > 1 ? 2 : 1;
    plan_family(plan, (uint32_t)graphics, graphics_queues);
    plan->graphics = (struct pl_vulkan_queue){(uint32_t)graphics, 1};

    if (compute >= 0) {
        plan_family(plan, (uint32_t)compute, 1);
        plan->compute = (struct pl_vulkan_queue){(uint32_t)compute, 1};
    }

    if (transfer >= 0) {
        plan_family(plan, (uint32_t)transfer, 1);
        plan->transfer = (struct pl_vulkan_queue){(uint32_t)transfer, 1};
    }

    for (uint32_t index = 0; index < count; index++) {
        if (families[index].queueFlags & VK_QUEUE_VIDEO_DECODE_BIT_KHR)
            plan_family(plan, index, 1);
    }

    return true;
}

static pl_vulkan import_device(
    struct flowa_gpu *gpu,
    const struct flowa_gpu_init *init
) {
    struct queue_plan queues = {0};

    if (!covers_wgpu_features(init->features) || !plan_queues(init, &queues))
        return NULL;

    enable_features(gpu, init);

    VkDeviceCreateInfo info = {
        .sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
        .pNext = &gpu->features.core,
        .queueCreateInfoCount = queues.count,
        .pQueueCreateInfos = queues.families,
        .enabledExtensionCount = (uint32_t)gpu->num_extensions,
        .ppEnabledExtensionNames = gpu->extensions,
    };
    PFN_vkCreateDevice create =
        (void *)init->get_proc_addr(init->instance, "vkCreateDevice");
    gpu->destroy_device =
        (void *)init->get_proc_addr(init->instance, "vkDestroyDevice");

    if (create(init->physical_device, &info, NULL, &gpu->device) != VK_SUCCESS) {
        gpu->device = VK_NULL_HANDLE;
        return NULL;
    }

    pl_vulkan vk = pl_vulkan_import(
        gpu->log,
        pl_vulkan_import_params(
            .instance = init->instance,
            .get_proc_addr = init->get_proc_addr,
            .phys_device = init->physical_device,
            .device = gpu->device,
            .extensions = gpu->extensions,
            .num_extensions = gpu->num_extensions,
            .queue_graphics = queues.graphics,
            .queue_compute = queues.compute,
            .queue_transfer = queues.transfer,
            .features = &gpu->features.core
        )
    );

    if (!vk) {
        video_gpu_destroy_vulkan(gpu);
        return NULL;
    }

    gpu->enabled_features = &gpu->features.core;
    return vk;
}

// The slower path for feature requests that import_device cannot reproduce
static pl_vulkan create_device(
    struct flowa_gpu *gpu,
    const struct flowa_gpu_init *init,
    const char **decoding,
    int decoding_count
) {
    enable_features(gpu, init);

    // Libplacebo merges wgpu's requests with the queried core features
    gpu->features.v13.pNext = init->features ? init->features->pNext : NULL;

    pl_vulkan vk = pl_vulkan_create(
        gpu->log,
        pl_vulkan_params(
            .instance = init->instance,
            .device = init->physical_device,
            .get_proc_addr = init->get_proc_addr,
            .extensions = init->extensions,
            .num_extensions = init->num_extensions,
            .opt_extensions = decoding,
            .num_opt_extensions = decoding_count,
            .features = &gpu->features.core,
            .queue_count = 1,
            .extra_queues = VK_QUEUE_VIDEO_DECODE_BIT_KHR
        )
    );

    if (!vk)
        return NULL;

    // Libplacebo owns these lists for the lifetime of the device
    int count = vk->num_extensions < FLOWA_MAX_DEVICE_EXTENSIONS
        ? vk->num_extensions
        : FLOWA_MAX_DEVICE_EXTENSIONS;
    for (int index = 0; index < count; index++)
        gpu->extensions[index] = vk->extensions[index];
    gpu->num_extensions = count;
    gpu->enabled_features = vk->features;

    return vk;
}

pl_vulkan video_gpu_create_vulkan(
    struct flowa_gpu *gpu,
    const struct flowa_gpu_init *init
) {
    int optional_count = 0;
    const char **optional = av_vk_get_optional_device_extensions(&optional_count);
    int decoding_count = 0;

    // Encoding extensions make FFmpeg add incompatible encode usage to decoder images
    for (int index = 0; index < optional_count; index++) {
        if (!strstr(optional[index], "video_encode"))
            optional[decoding_count++] = optional[index];
    }

    pl_vulkan vk = NULL;

    if (enable_extensions(gpu, init, optional, decoding_count))
        vk = import_device(gpu, init);

    if (!vk) {
        log_warning(gpu, "Creating the video device with every queue enabled");
        vk = create_device(gpu, init, optional, decoding_count);
    }

    // The array is FFmpeg's, but the extension names are static strings
    av_free(optional);
    return vk;
}

void video_gpu_destroy_vulkan(struct flowa_gpu *gpu) {
    pl_vulkan_destroy(&gpu->vk);

    if (gpu->device) {
        gpu->destroy_device(gpu->device, NULL);
        gpu->device = VK_NULL_HANDLE;
    }
}
