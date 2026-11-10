//! Every file format flowa recognizes, what kind of media it is, and whether this
//! build can open it

use std::path::Path;

/// How a file is shown: decoded to pixels, rendered from paths, or played
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Raster,
    Vector,
    Video,
}

struct Format {
    extensions: &'static [&'static str],
    kind: MediaKind,
    /// Whether this build includes the decoder
    enabled: bool,
}

const FORMATS: &[Format] = &[
    Format {
        extensions: &[
            "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "webp", "bmp", "ico",
            "tif", "tiff", "tga", "dds", "exr", "hdr", "ff", "qoi", "pnm", "pbm",
            "pgm", "ppm", "pam", "jxl",
        ],
        kind: MediaKind::Raster,
        enabled: true,
    },
    Format {
        extensions: &["avif"],
        kind: MediaKind::Raster,
        enabled: cfg!(feature = "avif"),
    },
    Format {
        extensions: &["heic", "heif"],
        kind: MediaKind::Raster,
        enabled: cfg!(feature = "heif"),
    },
    Format {
        extensions: &["svg", "svgz"],
        kind: MediaKind::Vector,
        enabled: true,
    },
    Format {
        extensions: &[
            "mp4", "m4v", "mkv", "webm", "mov", "avi", "mpg", "mpeg", "m2ts", "mts",
            "ts", "ogv",
        ],
        kind: MediaKind::Video,
        enabled: cfg!(video_playback),
    },
];

/// The kind of media a path holds by its extension, even when this build cannot open it
pub fn kind(path: &Path) -> Option<MediaKind> {
    format(path).map(|format| format.kind)
}

/// Whether this build can open the path
pub fn supported(path: &Path) -> bool {
    format(path).is_some_and(|format| format.enabled)
}

/// Every extension this build can open, for folder scans and file dialogs
pub fn supported_extensions() -> impl Iterator<Item = &'static str> {
    FORMATS
        .iter()
        .filter(|format| format.enabled)
        .flat_map(|format| format.extensions.iter().copied())
}

/// The file's format for display, such as `PNG` or `MKV`
pub fn label(path: &Path) -> Option<String> {
    let extension = path.extension().filter(|extension| !extension.is_empty())?;

    Some(extension.to_string_lossy().to_ascii_uppercase())
}

fn format(path: &Path) -> Option<&'static Format> {
    let extension = path.extension()?;

    FORMATS.iter().find(|format| {
        format
            .extensions
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}
