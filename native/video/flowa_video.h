// Primitive FFmpeg handles and libplacebo Vulkan interoperability for the Rust player
#pragma once

#include <stddef.h>
#include <stdint.h>
#include <vulkan/vulkan.h>

#define FLOWA_VIDEO_ABI 3
#define FLOWA_DECODER_SOFTWARE 3
#define FLOWA_VIDEO_SLOTS 3
#define FLOWA_VIDEO_ERROR_SIZE 768

struct flowa_gpu;
struct flowa_media;
struct flowa_stream;
struct flowa_packet;
struct flowa_decoder;
struct flowa_frame;

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

struct flowa_stream_info {
    double origin;
    double duration;
    double video_time_base;
    double audio_time_base;
    double frame_duration;
    double rotation;
    int has_audio;
    char codec[64];
};

struct flowa_frame_info {
    int64_t timestamp;
    int width;
    int height;
    int aspect_num;
    int aspect_den;
    double rotation;
};

struct flowa_audio_block {
    float *samples;
    int frames;
    int64_t timestamp;
    double resample_delay;
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
int flowa_gpu_render(
    struct flowa_gpu *gpu,
    const struct flowa_frame *frame,
    const struct flowa_stream *stream,
    const struct flowa_video_target *target,
    char *error,
    size_t error_size
);

// Media objects stay on their worker; stream snapshots and packets can transfer ownership
struct flowa_media *flowa_media_open(
    const char *path,
    int (*interrupt)(void *),
    void *userdata,
    struct flowa_stream_info *info,
    char *error,
    size_t error_size
);
void flowa_media_close(struct flowa_media *media);
struct flowa_stream *flowa_media_copy_stream(struct flowa_media *media);
void flowa_stream_free(struct flowa_stream *stream);
int flowa_media_read(
    struct flowa_media *media,
    struct flowa_packet **packet,
    int *kind,
    int *bytes,
    char *error,
    size_t error_size
);
int flowa_media_seek(
    struct flowa_media *media,
    int64_t timestamp,
    char *error,
    size_t error_size
);
struct flowa_packet *flowa_packet_clone(const struct flowa_packet *packet);
void flowa_packet_free(struct flowa_packet *packet);

// Backend values are 0=NVDEC, 1=Vulkan, 2=VA-API, 3=software; software accepts a NULL GPU
struct flowa_decoder *flowa_decoder_create(
    struct flowa_gpu *gpu,
    const struct flowa_stream *stream,
    int backend,
    char *error,
    size_t error_size
);
void flowa_decoder_free(struct flowa_decoder *decoder);
void flowa_decoder_flush(struct flowa_decoder *decoder);
int flowa_decoder_send(
    struct flowa_decoder *decoder,
    const struct flowa_packet *packet,
    char *error,
    size_t error_size
);
int flowa_decoder_receive(
    struct flowa_decoder *decoder,
    struct flowa_frame **frame,
    char *error,
    size_t error_size
);
void flowa_frame_free(struct flowa_frame *frame);
void flowa_frame_get_info(
    const struct flowa_frame *frame,
    struct flowa_frame_info *info
);

// Writes a small packed RGBA poster without initializing audio or a GPU
int flowa_video_thumbnail(
    const char *path,
    int (*interrupt)(void *),
    void *userdata,
    uint8_t *pixels,
    int max_width,
    int max_height,
    int *width,
    int *height,
    double *rotation,
    char *error,
    size_t error_size
);

int flowa_media_audio_send(
    struct flowa_media *media,
    const struct flowa_packet *packet,
    char *error,
    size_t error_size
);
int flowa_media_audio_receive(
    struct flowa_media *media,
    struct flowa_audio_block *block,
    char *error,
    size_t error_size
);
void flowa_audio_block_free(float *samples);
