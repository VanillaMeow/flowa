// Libplacebo helpers that C alone can reach: macros, header-implemented functions,
// and exported data
#pragma once

#include <libavformat/avformat.h>
#include <libplacebo/log.h>
#include <libplacebo/renderer.h>
#include <libplacebo/vulkan.h>
#include <stdbool.h>

// Libplacebo's log constructor is a macro naming a different symbol per API version
pl_log flowa_log_create(const struct pl_log_params *params);

const struct pl_vulkan_params *flowa_vulkan_default_params();
const char *const *flowa_vulkan_recommended_extensions(int *count);

// Imports a decoded frame with the HDR and rotation properties of its stream,
// reusing the given textures for software planes
bool flowa_frame_map(
    pl_gpu gpu,
    struct pl_frame *mapped,
    const AVFrame *frame,
    const AVStream *stream,
    pl_tex planes[4]
);
void flowa_frame_unmap(pl_gpu gpu, struct pl_frame *mapped);

// Converts a mapped frame to sRGB in an RGBA target with the default render settings
bool flowa_frame_render(
    pl_renderer renderer,
    const struct pl_frame *mapped,
    pl_tex target
);
