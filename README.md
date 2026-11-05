> [!WARNING]
> This project is current in heavy development. Please check back later.

# flowa

**flowa** is a native image and video viewer built with Rust, egui, and wgpu.
It supports animated images, JPEG XL, AVIF, and HEIC/HEIF alongside common image formats, with thumbnails, image details, and configurable themes.

Linux video playback uses FFmpeg hardware decoding, libplacebo Vulkan rendering,
and SDL3 audio, with seeking and volume controls. Video frames stay on the GPU;
software video decoding is disabled. See the [video setup guide](native/video/README.md).

> [!IMPORTANT]
> Flowa currently needs to be built from source. See [BUILDING.md](BUILDING.md)

## Notes

- Linux settings are saved in `~/.config/flowa/settings.json`, or under `$XDG_CONFIG_HOME` if set.
