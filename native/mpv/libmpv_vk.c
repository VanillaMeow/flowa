// Vulkan targets for mpv's existing GPU renderer and hardware decoder interop
#include <libplacebo/vulkan.h>

#include "flowa_vk.h"
#include "video/out/gpu/context.h"
#include "video/out/gpu/libmpv_gpu.h"
#include "video/out/placebo/ra_pl.h"
#include "video/out/placebo/utils.h"

struct target {
    VkImage image;
    pl_tex texture;
    struct ra_tex proxy;
};

struct priv {
    struct ra_ctx ra_ctx;
    pl_log log;
    pl_vulkan vulkan;
    struct target targets[FLOWA_VK_SLOTS];
    struct target *active;
    struct flowa_vk_target frame;
};

unsigned flowa_mpv_vulkan_abi_version(void)
{
    return 1;
}

static int init(struct libmpv_gpu_context *ctx, mpv_render_param *params)
{
    const struct flowa_vk_init *args =
        get_mpv_render_param(params, FLOWA_VK_INIT, NULL);
    if (!args || !args->instance || !args->device || !args->features)
        return MPV_ERROR_INVALID_PARAMETER;

    struct priv *p = ctx->priv = talloc_zero(NULL, struct priv);
    p->log = mppl_log_create(p, ctx->log);
    p->vulkan = pl_vulkan_import(p->log, pl_vulkan_import_params(
        .instance = args->instance,
        .phys_device = args->physical_device,
        .device = args->device,
        .get_proc_addr = args->get_proc_addr,
        .queue_graphics = { .index = args->queue_family, .count = 1 },
        .extensions = args->extensions,
        .num_extensions = args->num_extensions,
        .features = args->features,
    ));
    if (!p->vulkan)
        return MPV_ERROR_UNSUPPORTED;

    p->ra_ctx = (struct ra_ctx) {
        .global = ctx->global,
        .log = ctx->log,
        .ra = ra_create_pl(p->vulkan->gpu, ctx->log),
    };
    if (!p->ra_ctx.ra)
        return MPV_ERROR_VO_INIT_FAILED;

    ctx->ra_ctx = &p->ra_ctx;
    return 0;
}

static int wrap_fbo(struct libmpv_gpu_context *ctx, mpv_render_param *params,
                    struct ra_tex **out)
{
    struct priv *p = ctx->priv;
    const struct flowa_vk_target *args =
        get_mpv_render_param(params, FLOWA_VK_TARGET, NULL);
    if (!args || !args->image || !args->width || !args->height ||
        args->slot >= FLOWA_VK_SLOTS || !args->semaphore)
        return MPV_ERROR_INVALID_PARAMETER;

    pl_gpu gpu = p->vulkan->gpu;
    struct target *target = &p->targets[args->slot];
    if (target->image != args->image || !target->texture ||
        target->texture->params.w != args->width ||
        target->texture->params.h != args->height) {
        if (target->texture) {
            pl_gpu_finish(gpu);
            pl_tex_destroy(gpu, &target->texture);
        }

        target->image = args->image;
        target->texture = pl_vulkan_wrap(gpu, pl_vulkan_wrap_params(
            .image = args->image,
            .width = args->width,
            .height = args->height,
            .format = VK_FORMAT_R8G8B8A8_UNORM,
            .usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT |
                     VK_IMAGE_USAGE_SAMPLED_BIT |
                     VK_IMAGE_USAGE_TRANSFER_DST_BIT |
                     VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
        ));
        if (!target->texture)
            return MPV_ERROR_GENERIC;

        if (!mppl_wrap_tex(p->ra_ctx.ra, target->texture, &target->proxy))
            return MPV_ERROR_GENERIC;
    }

    // The host transitions the image before signaling the timeline value
    pl_vulkan_release_ex(gpu, pl_vulkan_release_params(
        .tex = target->texture,
        .layout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        .qf = VK_QUEUE_FAMILY_IGNORED,
        .semaphore = { args->semaphore, args->wait_value },
    ));

    p->active = target;
    p->frame = *args;
    *out = &target->proxy;
    return 0;
}

static void done_frame(struct libmpv_gpu_context *ctx, bool display_synced)
{
    struct priv *p = ctx->priv;

    // Restore the layout tracked by wgpu and signal completion without a CPU wait
    bool held = pl_vulkan_hold_ex(p->vulkan->gpu, pl_vulkan_hold_params(
        .tex = p->active->texture,
        .layout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        .qf = VK_QUEUE_FAMILY_IGNORED,
        .semaphore = { p->frame.semaphore, p->frame.signal_value },
    ));
    if (!held)
        MP_FATAL(ctx, "Could not return the Vulkan target to the host\n");

    pl_gpu_flush(p->vulkan->gpu);
    p->active = NULL;
}

static void destroy(struct libmpv_gpu_context *ctx)
{
    struct priv *p = ctx->priv;
    if (!p)
        return;

    if (p->vulkan) {
        pl_gpu_finish(p->vulkan->gpu);

        for (int n = 0; n < FLOWA_VK_SLOTS; n++)
            pl_tex_destroy(p->vulkan->gpu, &p->targets[n].texture);

        if (p->ra_ctx.ra)
            p->ra_ctx.ra->fns->destroy(p->ra_ctx.ra);

        pl_vulkan_destroy(&p->vulkan);
    }

    pl_log_destroy(&p->log);
}

const struct libmpv_gpu_context_fns libmpv_gpu_context_flowa_vk = {
    .api_name = FLOWA_VK_API,
    .init = init,
    .wrap_fbo = wrap_fbo,
    .done_frame = done_frame,
    .destroy = destroy,
};
