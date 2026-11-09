//! Catalog entries, supported formats, ordering, and navigation rules

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::settings::SortOrder;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "webp", "bmp", "ico", "tif",
    "tiff", "tga", "dds", "exr", "hdr", "ff", "qoi", "pnm", "pbm", "pgm", "ppm", "pam",
    "svg", "svgz", "jxl",
];

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "mpg", "mpeg", "m2ts", "mts", "ts",
    "ogv",
];

/// Every extension this build can open, for folder scans and file dialogs
pub fn supported_extensions() -> impl Iterator<Item = &'static str> {
    let video: &[&str] = if cfg!(video_playback) {
        VIDEO_EXTENSIONS
    } else {
        &[]
    };
    let avif: &[&str] = if cfg!(feature = "avif") {
        &["avif"]
    } else {
        &[]
    };
    let heif: &[&str] = if cfg!(feature = "heif") {
        &["heic", "heif"]
    } else {
        &[]
    };

    IMAGE_EXTENSIONS
        .iter()
        .chain(video)
        .chain(avif)
        .chain(heif)
        .copied()
}

pub fn is_video(path: &Path) -> bool {
    has_extension(path, VIDEO_EXTENSIONS.iter().copied())
}

pub fn supported(path: &Path) -> bool {
    has_extension(path, supported_extensions())
}

fn has_extension<'a>(
    path: &Path,
    mut candidates: impl Iterator<Item = &'a str>,
) -> bool {
    path.extension().is_some_and(|extension| {
        candidates.any(|candidate| extension.eq_ignore_ascii_case(candidate))
    })
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified: SystemTime,
}

// Scan data has no worker or filesystem dependencies
pub struct ScannedFolder {
    pub folder: PathBuf,
    pub selected: Option<PathBuf>,
    pub entries: Vec<Entry>,
}

pub fn sort(files: &mut [Entry], order: SortOrder, reverse: bool) {
    files.sort_by(|a, b| {
        let name = || {
            natord::compare_ignore_case(&a.name, &b.name)
                .then_with(|| a.path.cmp(&b.path))
        };
        let cmp = match order {
            SortOrder::Name => name(),
            SortOrder::Modified => b.modified.cmp(&a.modified).then_with(name),
            SortOrder::Size => b.size.cmp(&a.size).then_with(name),
        };
        if reverse { cmp.reverse() } else { cmp }
    });
}

pub fn next_index(
    current: usize,
    len: usize,
    delta: isize,
    wrap: bool,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let next = current as isize + delta;
    Some(if wrap {
        next.rem_euclid(len as isize) as usize
    } else {
        next.clamp(0, len as isize - 1) as usize
    })
}
