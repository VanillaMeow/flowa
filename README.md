# flowa

**flowa** is a native image viewer built with Rust, egui, and wgpu.
It supports animated images, JPEG XL, AVIF, and HEIC/HEIF alongside common image formats, with thumbnails, image details, and configurable themes.

> [!IMPORTANT]
> Flowa currently needs to be built from source. See [BUILDING.md](BUILDING.md)

## Notes

- Linux settings are saved in `~/.config/flowa/settings.json`, or under `$XDG_CONFIG_HOME` if set.
