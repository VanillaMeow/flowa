// GPU copies from decode-only images into sampleable Vulkan planes
#include "internal.h"

#include <stdlib.h>

struct video_transfer {
    VkCommandPool pool;
    VkQueue queue;
    PFN_vkAllocateCommandBuffers allocate;
    PFN_vkBeginCommandBuffer begin;
    PFN_vkEndCommandBuffer end;
    PFN_vkCmdPipelineBarrier2 barrier;
    PFN_vkCmdCopyImage copy;
    PFN_vkQueueSubmit2 submit;
    PFN_vkFreeCommandBuffers free;
    PFN_vkDestroyCommandPool destroy;
    PFN_vkWaitSemaphores wait;
};

static bool prepare_transfer(struct flowa_gpu *gpu) {
    if (gpu->transfer)
        return true;

    struct video_transfer *transfer = calloc(1, sizeof(*transfer));
    if (!transfer)
        return false;

    PFN_vkGetInstanceProcAddr get = gpu->vk->get_proc_addr;
    VkInstance instance = gpu->vk->instance;
    PFN_vkCreateCommandPool create = (void *)get(instance, "vkCreateCommandPool");
    PFN_vkGetDeviceQueue get_queue = (void *)get(instance, "vkGetDeviceQueue");
    transfer->allocate = (void *)get(instance, "vkAllocateCommandBuffers");
    transfer->begin = (void *)get(instance, "vkBeginCommandBuffer");
    transfer->end = (void *)get(instance, "vkEndCommandBuffer");
    transfer->barrier = (void *)get(instance, "vkCmdPipelineBarrier2");
    transfer->copy = (void *)get(instance, "vkCmdCopyImage");
    transfer->submit = (void *)get(instance, "vkQueueSubmit2");
    transfer->free = (void *)get(instance, "vkFreeCommandBuffers");
    transfer->destroy = (void *)get(instance, "vkDestroyCommandPool");
    transfer->wait = (void *)get(instance, "vkWaitSemaphores");

    if (!transfer->barrier)
        transfer->barrier = (void *)get(instance, "vkCmdPipelineBarrier2KHR");

    if (!transfer->submit)
        transfer->submit = (void *)get(instance, "vkQueueSubmit2KHR");

    if (
        !create || !get_queue || !transfer->allocate || !transfer->begin ||
        !transfer->end || !transfer->barrier || !transfer->copy ||
        !transfer->submit || !transfer->free || !transfer->destroy || !transfer->wait
    ) {
        free(transfer);
        return false;
    }

    VkCommandPoolCreateInfo info = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        .flags = VK_COMMAND_POOL_CREATE_TRANSIENT_BIT,
        .queueFamilyIndex = gpu->vk->queue_graphics.index,
    };
    if (create(gpu->vk->device, &info, NULL, &transfer->pool) != VK_SUCCESS) {
        free(transfer);
        return false;
    }

    get_queue(gpu->vk->device, info.queueFamilyIndex, 0, &transfer->queue);
    gpu->transfer = transfer;
    return true;
}

static bool prepare_images(
    const AVHWFramesContext *pool,
    AVVkFrame *frame,
    bool destination,
    uint32_t family,
    uint32_t *count,
    VkImageMemoryBarrier2 *barriers,
    VkSemaphoreSubmitInfo *waits,
    VkSemaphoreSubmitInfo *signals
) {
    const AVVulkanFramesContext *images = pool->hwctx;
    VkImageUsageFlags required_usage = destination ?
        VK_IMAGE_USAGE_TRANSFER_DST_BIT : VK_IMAGE_USAGE_TRANSFER_SRC_BIT;

    if (!(images->usage & required_usage))
        return false;

    for (
        int image = 0; image < AV_NUM_DATA_POINTERS && images->format[image];
        image++
    ) {
        if (
            frame->queue_family[image] != VK_QUEUE_FAMILY_IGNORED &&
            frame->queue_family[image] != family &&
            frame->layout[image] != VK_IMAGE_LAYOUT_UNDEFINED
        )
            return false;

        uint32_t index = (*count)++;
        barriers[index] = (VkImageMemoryBarrier2){
            .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2,
            // Timeline waits make the previous queue's writes available
            .srcStageMask = VK_PIPELINE_STAGE_2_NONE,
            .srcAccessMask = VK_ACCESS_2_NONE,
            .dstStageMask = VK_PIPELINE_STAGE_2_TRANSFER_BIT,
            .dstAccessMask =
                destination ? VK_ACCESS_2_TRANSFER_WRITE_BIT :
                VK_ACCESS_2_TRANSFER_READ_BIT,
            .oldLayout = frame->layout[image],
            .newLayout =
                destination ? VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL :
                VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .image = frame->img[image],
            .subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1},
        };
        waits[index] = (VkSemaphoreSubmitInfo){
            .sType = VK_STRUCTURE_TYPE_SEMAPHORE_SUBMIT_INFO,
            .semaphore = frame->sem[image],
            .value = frame->sem_value[image],
            .stageMask = VK_PIPELINE_STAGE_2_ALL_COMMANDS_BIT,
        };
        signals[index] = waits[index];
        signals[index].value++;
    }

    return true;
}

static void update_images(
    const AVHWFramesContext *pool,
    AVVkFrame *frame,
    bool destination,
    uint32_t family
) {
    const AVVulkanFramesContext *images = pool->hwctx;

    for (
        int image = 0; image < AV_NUM_DATA_POINTERS && images->format[image];
        image++
    ) {
        frame->layout[image] =
            destination ? VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL :
            VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
        frame->access[image] =
            destination ? VK_ACCESS_2_TRANSFER_WRITE_BIT :
            VK_ACCESS_2_TRANSFER_READ_BIT;
        frame->sem_value[image]++;

        if (frame->queue_family[image] != VK_QUEUE_FAMILY_IGNORED)
            frame->queue_family[image] = family;
    }
}

static void copy_planes(
    const struct video_transfer *transfer,
    VkCommandBuffer command,
    const AVHWFramesContext *source_pool,
    const AVHWFramesContext *destination_pool,
    const AVVkFrame *source,
    const AVVkFrame *destination
) {
    const AVVulkanFramesContext *source_images = source_pool->hwctx;
    const AVVulkanFramesContext *destination_images = destination_pool->hwctx;
    const AVPixFmtDescriptor *format = av_pix_fmt_desc_get(source_pool->sw_format);
    int planes = av_pix_fmt_count_planes(source_pool->sw_format);
    bool source_multiplane = planes > 1 && !source_images->format[1];
    bool destination_multiplane = planes > 1 && !destination_images->format[1];

    for (int plane = 0; plane < planes; plane++) {
        VkImageAspectFlags aspect = VK_IMAGE_ASPECT_PLANE_0_BIT << plane;
        VkImageCopy region = {
            .srcSubresource = {
                .aspectMask = source_multiplane ? aspect : VK_IMAGE_ASPECT_COLOR_BIT,
                .layerCount = 1,
            },
            .dstSubresource = {
                .aspectMask =
                    destination_multiplane ? aspect : VK_IMAGE_ASPECT_COLOR_BIT,
                .layerCount = 1,
            },
            .extent = {
                .width = AV_CEIL_RSHIFT(
                    source_pool->width,
                    plane ? format->log2_chroma_w : 0
                    ),
                .height = AV_CEIL_RSHIFT(
                    source_pool->height,
                    plane ? format->log2_chroma_h : 0
                    ),
                .depth = 1,
            },
        };
        transfer->copy(
            command,
            source->img[source_multiplane ? 0 : plane],
            VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            destination->img[destination_multiplane ? 0 : plane],
            VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
            1,
            &region
        );
    }
}

AVFrame *video_gpu_transfer_vulkan(
    struct flowa_gpu *gpu,
    const AVFrame *source,
    VkCommandBuffer *command
) {
    AVBufferRef *pool = video_gpu_frame_pool(gpu, source);
    AVFrame *frame = av_frame_alloc();
    if (!pool || !frame || !prepare_transfer(gpu))
        goto error;

    if (
        av_hwframe_get_buffer(pool, frame, 0) < 0 ||
        av_frame_copy_props(frame, source) < 0
    )
        goto error;

    frame->width = source->width;
    frame->height = source->height;
    struct video_transfer *transfer = gpu->transfer;
    VkCommandBufferAllocateInfo allocation = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        .commandPool = transfer->pool,
        .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        .commandBufferCount = 1,
    };
    if (transfer->allocate(gpu->vk->device, &allocation, command) != VK_SUCCESS) {
        *command = VK_NULL_HANDLE;
        goto error;
    }

    VkCommandBufferBeginInfo begin = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
        .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
    };
    if (transfer->begin(*command, &begin) != VK_SUCCESS)
        goto error;

    AVHWFramesContext *source_pool = (void *)source->hw_frames_ctx->data;
    AVHWFramesContext *destination_pool = (void *)frame->hw_frames_ctx->data;
    AVVulkanFramesContext *source_images = source_pool->hwctx;
    AVVulkanFramesContext *destination_images = destination_pool->hwctx;
    AVVkFrame *source_frame = (void *)source->data[0];
    AVVkFrame *destination_frame = (void *)frame->data[0];
    source_images->lock_frame(source_pool, source_frame);
    destination_images->lock_frame(destination_pool, destination_frame);

    VkImageMemoryBarrier2 barriers[2 * AV_NUM_DATA_POINTERS];
    VkSemaphoreSubmitInfo waits[2 * AV_NUM_DATA_POINTERS];
    VkSemaphoreSubmitInfo signals[2 * AV_NUM_DATA_POINTERS];
    uint32_t count = 0;
    uint32_t family = gpu->vk->queue_graphics.index;
    bool ready = prepare_images(
        source_pool,
        source_frame,
        false,
        family,
        &count,
        barriers,
        waits,
        signals
        ) &&
        prepare_images(
        destination_pool,
        destination_frame,
        true,
        family,
        &count,
        barriers,
        waits,
        signals
        );
    VkResult result = VK_ERROR_INITIALIZATION_FAILED;

    if (ready) {
        VkDependencyInfo dependency = {
            .sType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO,
            .imageMemoryBarrierCount = count,
            .pImageMemoryBarriers = barriers,
        };
        transfer->barrier(*command, &dependency);
        copy_planes(
            transfer,
            *command,
            source_pool,
            destination_pool,
            source_frame,
            destination_frame
        );
        result = transfer->end(*command);
    }

    if (result == VK_SUCCESS) {
        VkCommandBufferSubmitInfo buffer = {
            .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_SUBMIT_INFO,
            .commandBuffer = *command,
        };
        VkSubmitInfo2 submit = {
            .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO_2,
            .waitSemaphoreInfoCount = count,
            .pWaitSemaphoreInfos = waits,
            .commandBufferInfoCount = 1,
            .pCommandBufferInfos = &buffer,
            .signalSemaphoreInfoCount = count,
            .pSignalSemaphoreInfos = signals,
        };
        gpu->vk->lock_queue(gpu->vk, family, 0);
        result = transfer->submit(transfer->queue, 1, &submit, VK_NULL_HANDLE);
        gpu->vk->unlock_queue(gpu->vk, family, 0);
    }

    if (result == VK_SUCCESS) {
        update_images(source_pool, source_frame, false, family);
        update_images(destination_pool, destination_frame, true, family);
    }

    destination_images->unlock_frame(destination_pool, destination_frame);
    source_images->unlock_frame(source_pool, source_frame);

    if (result == VK_SUCCESS)
        return frame;

error:
    if (*command) {
        gpu->transfer->free(gpu->vk->device, gpu->transfer->pool, 1, command);
        *command = VK_NULL_HANDLE;
    }

    av_frame_free(&frame);
    return NULL;
}

void video_gpu_free_transfer(
    struct flowa_gpu *gpu,
    const AVFrame *frame,
    VkCommandBuffer command
) {
    if (!command)
        return;

    AVHWFramesContext *pool = (void *)frame->hw_frames_ctx->data;
    AVVulkanFramesContext *images = pool->hwctx;
    AVVkFrame *vulkan = (void *)frame->data[0];
    uint32_t count = 0;

    while (count < AV_NUM_DATA_POINTERS && images->format[count])
        count++;

    // Rendering has normally completed; error cleanup must also wait for the copy
    VkSemaphoreWaitInfo wait = {
        .sType = VK_STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO,
        .semaphoreCount = count,
        .pSemaphores = vulkan->sem,
        .pValues = vulkan->sem_value,
    };
    gpu->transfer->wait(gpu->vk->device, &wait, UINT64_MAX);
    gpu->transfer->free(gpu->vk->device, gpu->transfer->pool, 1, &command);
}

void video_gpu_destroy_transfer(struct flowa_gpu *gpu) {
    if (!gpu->transfer)
        return;

    gpu->transfer->destroy(gpu->vk->device, gpu->transfer->pool, NULL);
    free(gpu->transfer);
    gpu->transfer = NULL;
}
