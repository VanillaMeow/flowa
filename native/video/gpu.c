// Shared Vulkan device creation and hardware and software frame rendering
#include "internal.h"

#include <libplacebo/utils/libav.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int video_error(char *error, size_t size, const char *format, ...) {
    va_list arguments;
    va_start(arguments, format);
    vsnprintf(error, size, format, arguments);
    va_end(arguments);

    return -1;
}

struct AVBufferRef *flowa_gpu_create_decoder_device(
    struct flowa_gpu *gpu,
    int backend
) {
    AVBufferRef *device = NULL;

    if (backend == 1)
        return gpu->video_queue && gpu->av_device ? av_buffer_ref(gpu->av_device)
                                                  : NULL;

    if (backend == 0 && gpu->av_device)
        av_hwdevice_ctx_create_derived(
            &device,
            AV_HWDEVICE_TYPE_CUDA,
            gpu->av_device,
            0
        );

    if (backend == 2 && gpu->render_node[0])
        av_hwdevice_ctx_create(
            &device,
            AV_HWDEVICE_TYPE_VAAPI,
            gpu->render_node,
            NULL,
            0
        );

    return device;
}

static void gpu_log(void *userdata, enum pl_log_level level, const char *message) {
    struct flowa_gpu *gpu = userdata;

    if (gpu->logger)
        gpu->logger(level, message);
}

static bool has_extension(pl_vulkan vk, const char *name) {
    for (int i = 0; i < vk->num_extensions; i++) {
        if (!strcmp(vk->extensions[i], name))
            return true;
    }

    return false;
}

static void create_ffmpeg_device(struct flowa_gpu *gpu) {
    AVBufferRef *reference = av_hwdevice_ctx_alloc(AV_HWDEVICE_TYPE_VULKAN);
    if (!reference)
        return;

    AVHWDeviceContext *device = (void *)reference->data;
    AVVulkanDeviceContext *context = device->hwctx;
    context->inst = gpu->vk->instance;
    context->phys_dev = gpu->vk->phys_device;
    context->act_dev = gpu->vk->device;
    context->get_proc_addr = gpu->vk->get_proc_addr;
    context->device_features = *gpu->vk->features;
    context->enabled_dev_extensions = gpu->vk->extensions;
    context->nb_enabled_dev_extensions = gpu->vk->num_extensions;

    PFN_vkGetPhysicalDeviceQueueFamilyProperties get_queues =
        (void *)gpu->vk->get_proc_addr(
            context->inst,
            "vkGetPhysicalDeviceQueueFamilyProperties"
        );
    uint32_t count = 0;
    get_queues(context->phys_dev, &count, NULL);
    VkQueueFamilyProperties *queues = calloc(count, sizeof(*queues));
    if (!queues) {
        av_buffer_unref(&reference);
        return;
    }

    get_queues(context->phys_dev, &count, queues);
    for (uint32_t i = 0; i < count && context->nb_qf < 64; i++) {
        bool enabled =
            i == gpu->vk->queue_graphics.index ||
            (gpu->vk->queue_compute.count && i == gpu->vk->queue_compute.index) ||
            (gpu->vk->queue_transfer.count && i == gpu->vk->queue_transfer.index);
        bool video = queues[i].queueFlags & VK_QUEUE_VIDEO_DECODE_BIT_KHR;

        if (!enabled && !video)
            continue;

        context->qf[context->nb_qf++] = (AVVulkanDeviceQueueFamily){
            .idx = i,
            .num = 1,
            .flags = queues[i].queueFlags,
        };
        gpu->video_queue |= video;
    }
    free(queues);

    if (av_hwdevice_ctx_init(reference) < 0) {
        av_buffer_unref(&reference);
        return;
    }

    gpu->av_device = reference;
}

uint32_t flowa_video_abi_version(void) {
    return FLOWA_VIDEO_ABI;
}

struct flowa_gpu *flowa_gpu_create(
    const struct flowa_gpu_init *init,
    char *error,
    size_t error_size
) {
    struct flowa_gpu *gpu = calloc(1, sizeof(*gpu));
    if (!gpu) {
        snprintf(error, error_size, "Cannot allocate the native video device");
        return NULL;
    }

    gpu->logger = init->log;
    gpu->log = pl_log_create(
        PL_API_VER,
        pl_log_params(
            .log_cb = gpu_log,
            .log_priv = gpu,
            .log_level = PL_LOG_WARN
        )
    );

    // Query core features before appending wgpu's additional feature requests
    VkPhysicalDeviceVulkan13Features v13 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
    };
    VkPhysicalDeviceVulkan12Features v12 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES,
        .pNext = &v13,
    };
    VkPhysicalDeviceVulkan11Features v11 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES,
        .pNext = &v12,
    };
    VkPhysicalDeviceVideoMaintenance1FeaturesKHR video = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_MAINTENANCE_1_FEATURES_KHR,
        .pNext = &v11,
    };
#ifdef VK_KHR_video_maintenance2
    VkPhysicalDeviceVideoMaintenance2FeaturesKHR video2 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_MAINTENANCE_2_FEATURES_KHR,
        .pNext = &video,
    };
#endif
#ifdef VK_KHR_video_decode_vp9
    VkPhysicalDeviceVideoDecodeVP9FeaturesKHR vp9 = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_DECODE_VP9_FEATURES_KHR,
        .pNext = &video,
    };
#endif
    VkPhysicalDeviceFeatures2 features = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
        .pNext = &video,
    };
#ifdef VK_KHR_video_maintenance2
    features.pNext = &video2;
#endif
#ifdef VK_KHR_video_decode_vp9
    vp9.pNext = features.pNext;
    features.pNext = &vp9;
#endif
    PFN_vkGetPhysicalDeviceFeatures2 get_features =
        (void *)init->get_proc_addr(init->instance, "vkGetPhysicalDeviceFeatures2");
    get_features(init->physical_device, &features);
    v13.pNext = init->features ? init->features->pNext : NULL;

    int optional_count = 0;
    const char **optional = av_vk_get_optional_device_extensions(&optional_count);
    int decode_count = 0;

    // Encoding extensions make FFmpeg add incompatible encode usage to decoder images
    for (int index = 0; index < optional_count; index++) {
        if (!strstr(optional[index], "video_encode"))
            optional[decode_count++] = optional[index];
    }
    gpu->vk = pl_vulkan_create(
        gpu->log,
        pl_vulkan_params(
            .instance = init->instance,
            .device = init->physical_device,
            .get_proc_addr = init->get_proc_addr,
            .extensions = init->extensions,
            .num_extensions = init->num_extensions,
            .opt_extensions = optional,
            .num_opt_extensions = decode_count,
            .features = &features,
            .extra_queues = VK_QUEUE_VIDEO_DECODE_BIT_KHR
        )
    );
    av_free(optional);

    if (!gpu->vk) {
        snprintf(error, error_size, "Cannot create the shared Vulkan video device");
        flowa_gpu_destroy(gpu);
        return NULL;
    }

    gpu->renderer = pl_renderer_create(gpu->log, gpu->vk->gpu);
    if (!gpu->renderer) {
        snprintf(error, error_size, "Cannot create the libplacebo renderer");
        flowa_gpu_destroy(gpu);
        return NULL;
    }

    create_ffmpeg_device(gpu);
    gpu->video_queue &=
        has_extension(gpu->vk, VK_KHR_VIDEO_DECODE_QUEUE_EXTENSION_NAME);

    if (has_extension(gpu->vk, VK_EXT_PHYSICAL_DEVICE_DRM_EXTENSION_NAME)) {
        VkPhysicalDeviceDrmPropertiesEXT drm = {
            .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT,
        };
        VkPhysicalDeviceProperties2 properties = {
            .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2,
            .pNext = &drm,
        };
        PFN_vkGetPhysicalDeviceProperties2 get_properties =
            (void *)
            init->get_proc_addr(init->instance, "vkGetPhysicalDeviceProperties2");
        get_properties(init->physical_device, &properties);

        if (drm.hasRender)
            snprintf(
                gpu->render_node,
                sizeof(gpu->render_node),
                "/dev/dri/renderD%lld",
                (long long)drm.renderMinor
            );
    }

    return gpu;
}

void flowa_gpu_get_info(struct flowa_gpu *gpu, struct flowa_gpu_info *info) {
    *info = (struct flowa_gpu_info){
        .device = gpu->vk->device,
        .graphics_family = gpu->vk->queue_graphics.index,
        .extensions = gpu->vk->extensions,
        .num_extensions = gpu->vk->num_extensions,
        .features = gpu->vk->features,
    };
}

void video_gpu_clear_targets(struct flowa_gpu *gpu) {
    pl_gpu_finish(gpu->vk->gpu);
    video_gpu_collect(gpu, true);

    for (int i = 0; i < FLOWA_VIDEO_SLOTS; i++) {
        pl_tex_destroy(gpu->vk->gpu, &gpu->targets[i]);
        gpu->images[i] = VK_NULL_HANDLE;
    }
}

void flowa_gpu_retire_targets(struct flowa_gpu *gpu) {
    video_gpu_clear_targets(gpu);
}

void flowa_gpu_reset_frames(struct flowa_gpu *gpu) {
    // Rust calls this before dropping decoder frames on stop or file replacement
    pl_gpu_finish(gpu->vk->gpu);
    video_gpu_collect(gpu, true);
    av_buffer_unref(&gpu->upload_pool);
    gpu->upload_device = NULL;

    for (int plane = 0; plane < 4; plane++)
        pl_tex_destroy(gpu->vk->gpu, &gpu->software_planes[plane]);
}

void video_gpu_collect(struct flowa_gpu *gpu, bool finish) {
    PFN_vkGetSemaphoreCounterValue counter =
        (void *)gpu->vk->get_proc_addr(gpu->vk->instance, "vkGetSemaphoreCounterValue");
    struct gpu_flight **cursor = &gpu->flights;

    while (*cursor) {
        struct gpu_flight *flight = *cursor;
        uint64_t value = 0;
        VkResult result = counter(gpu->vk->device, flight->semaphore, &value);

        if (
            !finish && result != VK_ERROR_DEVICE_LOST &&
            (result != VK_SUCCESS || value < flight->value)
        ) {
            cursor = &flight->next;
            continue;
        }

        *cursor = flight->next;
        video_gpu_free_transfer(gpu, flight->converted, flight->transfer);
        av_frame_free(&flight->source);
        av_frame_free(&flight->converted);
        free(flight);
        gpu->flight_count--;
    }
}

void flowa_gpu_destroy(struct flowa_gpu *gpu) {
    if (!gpu)
        return;

    if (gpu->vk) {
        video_gpu_clear_targets(gpu);
        flowa_gpu_reset_frames(gpu);
        video_gpu_destroy_transfer(gpu);
        av_buffer_unref(&gpu->av_device);
        pl_renderer_destroy(&gpu->renderer);
        pl_vulkan_destroy(&gpu->vk);
    }

    pl_log_destroy(&gpu->log);
    free(gpu);
}

AVBufferRef *video_gpu_frame_pool(struct flowa_gpu *gpu, const AVFrame *source) {
    const AVHWFramesContext *source_pool = (void *)source->hw_frames_ctx->data;

    if (
        gpu->upload_pool && gpu->upload_format == source_pool->sw_format &&
        gpu->upload_width == source_pool->width &&
        gpu->upload_height == source_pool->height &&
        gpu->upload_device == source_pool->device_ctx
    )
        return gpu->upload_pool;

    av_buffer_unref(&gpu->upload_pool);
    if (!gpu->av_device)
        return NULL;

    AVBufferRef *pool = av_hwframe_ctx_alloc(gpu->av_device);
    if (!pool)
        return NULL;

    AVHWFramesContext *frames = (void *)pool->data;
    AVVulkanFramesContext *vulkan = frames->hwctx;
    frames->format = AV_PIX_FMT_VULKAN;
    frames->sw_format = source_pool->sw_format;
    frames->width = source_pool->width;
    frames->height = source_pool->height;
    vulkan->flags = AV_VK_FRAME_FLAG_DISABLE_MULTIPLANE;
    vulkan->usage = VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT |
        VK_IMAGE_USAGE_TRANSFER_SRC_BIT;

    if (av_hwframe_ctx_init(pool) < 0) {
        av_buffer_unref(&pool);
        return NULL;
    }

    gpu->upload_format = frames->sw_format;
    gpu->upload_width = frames->width;
    gpu->upload_height = frames->height;
    gpu->upload_device = source_pool->device_ctx;
    gpu->upload_pool = pool;
    return pool;
}

static AVFrame *map_cuda_frame(struct flowa_gpu *gpu, const AVFrame *source) {
    AVBufferRef *pool = video_gpu_frame_pool(gpu, source);
    AVFrame *frame = av_frame_alloc();
    if (!pool || !frame)
        goto error;

    int result = av_hwframe_get_buffer(pool, frame, 0);
    if (result < 0)
        goto error;

    // FFmpeg performs a device-to-device CUDA copy with external GPU semaphores
    result = av_hwframe_transfer_data(frame, source, 0);
    if (result < 0)
        goto error;

    if (av_frame_copy_props(frame, source) < 0)
        goto error;
    frame->width = source->width;
    frame->height = source->height;
    return frame;

error:
    pl_gpu_finish(gpu->vk->gpu);
    av_frame_free(&frame);
    return NULL;
}

static void free_converted_frame(
    struct flowa_gpu *gpu,
    AVFrame **frame,
    VkCommandBuffer command
) {
    video_gpu_free_transfer(gpu, *frame, command);
    av_frame_free(frame);
}

int flowa_gpu_render(
    struct flowa_gpu *gpu,
    const struct AVFrame *frame,
    const struct AVStream *stream,
    const struct flowa_video_target *target,
    char *error,
    size_t error_size
) {
    const AVFrame *current = (const void *)frame;
    if (!current || target->slot >= FLOWA_VIDEO_SLOTS)
        return 0;

    pl_gpu device = gpu->vk->gpu;
    video_gpu_collect(gpu, false);
    if (gpu->flight_count >= 32) {
        return video_error(
            error,
            error_size,
            "The GPU is not completing video submissions"
        );
    }

    pl_tex *texture = &gpu->targets[target->slot];
    if (
        gpu->images[target->slot] != target->image || !*texture ||
        (uint32_t)(*texture)->params.w != target->width ||
        (uint32_t)(*texture)->params.h != target->height
    ) {
        if (*texture) {
            pl_gpu_finish(device);
            pl_tex_destroy(device, texture);
        }

        *texture = pl_vulkan_wrap(
            device,
            pl_vulkan_wrap_params(
                .image = target->image,
                .width = target->width,
                .height = target->height,
                .format = VK_FORMAT_R8G8B8A8_UNORM,
                .usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT |
                    VK_IMAGE_USAGE_SAMPLED_BIT |
                    VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                    VK_IMAGE_USAGE_TRANSFER_DST_BIT
            )
        );
        gpu->images[target->slot] = target->image;
        if (!*texture)
            return -1;
    }

    AVFrame *converted = NULL;
    VkCommandBuffer transfer = VK_NULL_HANDLE;
    const AVFrame *source = current;
    if (source->format == AV_PIX_FMT_CUDA) {
        converted = map_cuda_frame(gpu, source);
        if (!converted) {
            return video_error(
                error,
                error_size,
                "Cannot transfer the NVDEC frame to Vulkan without CPU readback"
            );
        }
        source = converted;
    }

    if (source->format == AV_PIX_FMT_VULKAN) {
        const AVHWFramesContext *pool = (void *)source->hw_frames_ctx->data;
        const AVVulkanFramesContext *images = pool->hwctx;
        int plane_count = av_pix_fmt_count_planes(pool->sw_format);
        bool needs_plane_views = plane_count > 1 &&
            !images->format[1] &&
            !(images->img_flags & VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT);
        bool needs_plane_copy = needs_plane_views ||
            !(images->usage & VK_IMAGE_USAGE_SAMPLED_BIT);

        if (needs_plane_copy) {
            converted = video_gpu_transfer_vulkan(gpu, source, &transfer);
            if (!converted) {
                return video_error(
                    error,
                    error_size,
                    "Cannot copy Vulkan decode planes without CPU readback"
                );
            }

            source = converted;
        }
    }

    struct pl_frame input = {0};
    if (
        !pl_map_avframe_ex(
            device,
            &input,
            pl_avframe_params(
                .frame = source,
                .tex = gpu->software_planes
            )
        )
    ) {
        pl_gpu_finish(device);
        free_converted_frame(gpu, &converted, transfer);
        return video_error(
            error,
            error_size,
            "Cannot map the video frame into the Vulkan renderer"
        );
    }
    pl_frame_copy_stream_props(&input, stream);

    struct gpu_flight *flight = calloc(1, sizeof(*flight));
    if (!flight) {
        pl_gpu_finish(device);
        pl_unmap_avframe(device, &input);
        free_converted_frame(gpu, &converted, transfer);
        return -1;
    }
    flight->source = av_frame_clone(current);
    if (!flight->source) {
        pl_gpu_finish(device);
        pl_unmap_avframe(device, &input);
        free_converted_frame(gpu, &converted, transfer);
        free(flight);
        return video_error(error, error_size, "Cannot retain an in-flight video frame");
    }
    flight->converted = converted;
    flight->transfer = transfer;
    flight->semaphore = target->semaphore;
    flight->value = target->signal_value;
    flight->next = gpu->flights;
    gpu->flights = flight;
    gpu->flight_count++;

    pl_vulkan_release_ex(
        device,
        pl_vulkan_release_params(
            .tex = *texture,
            .layout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            .qf = VK_QUEUE_FAMILY_IGNORED,
            .semaphore = {target->semaphore, target->wait_value}
        )
    );

    struct pl_frame output = {
        .num_planes = 1,
        .planes = {
            {.texture = *texture, .components = 4, .component_mapping = {0, 1, 2, 3}},
        },
        .repr = pl_color_repr_rgb,
        .color = pl_color_space_srgb,
    };
    bool rendered =
        pl_render_image(gpu->renderer, &input, &output, &pl_render_default_params);
    pl_unmap_avframe(device, &input);

    // Always return ownership, even when a shader or hardware import fails
    bool held = pl_vulkan_hold_ex(
        device,
        pl_vulkan_hold_params(
            .tex = *texture,
            .layout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            .qf = VK_QUEUE_FAMILY_IGNORED,
            .semaphore = {target->semaphore, target->signal_value}
        )
    );
    pl_gpu_flush(device);

    if (!rendered || !held) {
        return video_error(
            error,
            error_size,
            "The Vulkan video render submission failed"
        );
    }

    return 1;
}
