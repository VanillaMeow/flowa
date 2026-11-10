// The parts of libplacebo that Rust cannot call or link directly
#include "placebo.h"

#include <libplacebo/utils/libav.h>

pl_log flowa_log_create(const struct pl_log_params *params) {
    return pl_log_create(PL_API_VER, params);
}

const struct pl_vulkan_params *flowa_vulkan_default_params() {
    return &pl_vulkan_default_params;
}

const char *const *flowa_vulkan_recommended_extensions(int *count) {
    *count = pl_vulkan_num_recommended_extensions;

    return pl_vulkan_recommended_extensions;
}

bool flowa_frame_map(
    pl_gpu gpu,
    struct pl_frame *mapped,
    const AVFrame *frame,
    const AVStream *stream,
    pl_tex planes[4]
) {
    const struct pl_avframe_params *params =
        pl_avframe_params(.frame = frame, .tex = planes);

    if (!pl_map_avframe_ex(gpu, mapped, params))
        return false;

    pl_frame_copy_stream_props(mapped, stream);

    return true;
}

void flowa_frame_unmap(pl_gpu gpu, struct pl_frame *mapped) {
    pl_unmap_avframe(gpu, mapped);
}

bool flowa_frame_render(
    pl_renderer renderer,
    const struct pl_frame *mapped,
    pl_tex target
) {
    struct pl_frame output = {
        .num_planes = 1,
        .planes = {
            {.texture = target, .components = 4, .component_mapping = {0, 1, 2, 3}},
        },
        .repr = pl_color_repr_rgb,
        .color = pl_color_space_srgb,
    };

    return pl_render_image(renderer, mapped, &output, &pl_render_default_params);
}
