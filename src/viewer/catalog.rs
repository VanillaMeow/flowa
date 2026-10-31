//! Catalog entries, supported formats, ordering, and navigation rules

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::settings::SortOrder;

pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "webp", "bmp", "ico", "tif",
    "tiff", "tga", "dds", "exr", "hdr", "ff", "qoi", "pnm", "pbm", "pgm", "ppm", "pam",
    "svg", "svgz", "jxl",
];

pub fn supported(path: &Path) -> bool {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str())
        || (cfg!(feature = "avif") && ext == "avif")
        || (cfg!(feature = "heif") && matches!(ext.as_str(), "heic" | "heif"))
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified: SystemTime,
}

// Scan data has no worker or filesystem dependencies
pub(crate) struct ScannedFolder {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_handles_edges_and_empty_folders() {
        assert_eq!(next_index(0, 0, 1, true), None);
        assert_eq!(next_index(0, 3, -1, true), Some(2));
        assert_eq!(next_index(2, 3, 1, false), Some(2));
    }
}
