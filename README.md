> [!WARNING]
> This project is current in heavy development. Please check back later.

# flowa

**flowa** is a native image and video viewer built with Rust, egui, and wgpu.
It supports animated images, JPEG XL, AVIF, and HEIC/HEIF alongside common image formats, with thumbnails, image details, and configurable themes.

Linux video playback uses a custom libmpv Vulkan backend with shared GPU textures,
audio, seeking, and volume controls. See the [video setup guide](native/mpv/README.md).

> [!IMPORTANT]
> Flowa currently needs to be built from source. See [BUILDING.md](BUILDING.md)

## Notes

- Linux settings are saved in `~/.config/flowa/settings.json`, or under `$XDG_CONFIG_HOME` if set.
