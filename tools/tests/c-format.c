// Keep wrapped headers, signatures, and nested calls vertically indented
static const AVCodec *hardware_codec(
    enum AVCodecID id,
    enum AVHWDeviceType type,
    enum AVPixelFormat *format
) {
    void *iterator = NULL;
    const AVCodec *codec;

    while ((codec = av_codec_iterate(&iterator))) {
        if (codec->id != id || !av_codec_is_decoder(codec))
            continue;

        const AVCodecHWConfig *configuration;
        for (
            int index = 0;
            (configuration = avcodec_get_hw_config(codec, index));
            index++
        ) {
            if (
                configuration->device_type == type &&
                configuration->methods & AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX
            ) {
                *format = configuration->pix_fmt;
                return codec;
            }
        }
    }

    return NULL;
}

static void configure(struct flowa_gpu *gpu) {
    gpu->vk = pl_vulkan_create(
        gpu->log,
        pl_vulkan_params(
            .instance = init->instance,
            .device = init->physical_device,
            .get_proc_addr = init->get_proc_addr
        )
    );
}
