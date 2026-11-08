> [!WARNING]
> This project is currently in heavy development. Please check back later.

# flowa

**flowa** is a native image and video viewer built with Rust, egui, and wgpu.
It supports animated images, JPEG XL, AVIF, and HEIC/HEIF alongside common image formats, with thumbnails, image details, and configurable themes.

On Linux it also plays video with hardware decoding, seeking, and volume controls.

> [!IMPORTANT]
> Flowa currently needs to be built from source. See [BUILDING.md](BUILDING.md)

## Notes

- Linux settings are saved in `~/.config/flowa/settings.json`, or under `$XDG_CONFIG_HOME` if set.
