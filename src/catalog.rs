use crate::settings::SortOrder;
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "webp", "bmp", "ico", "tif", "tiff", "tga",
    "dds", "exr", "hdr", "ff", "qoi", "pnm", "pbm", "pgm", "ppm", "pam", "svg", "svgz", "jxl",
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
impl Entry {
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let metadata = path
            .metadata()
            .with_context(|| format!("Cannot read {}", path.display()))?;
        ensure!(metadata.is_file(), "This path is not a file");
        Ok(Self {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            path,
        })
    }
}
pub fn scan(folder: &Path) -> Result<Vec<Entry>> {
    let mut files = Vec::new();
    for item in
        std::fs::read_dir(folder).with_context(|| format!("Cannot open {}", folder.display()))?
    {
        let item = item.context("Cannot read folder entry")?;
        let path = item.path();
        if supported(&path)
            && let Ok(entry) = Entry::from_path(path)
        {
            files.push(entry);
        }
    }
    Ok(files)
}
pub fn sort(files: &mut [Entry], order: SortOrder, reverse: bool) {
    files.sort_by(|a, b| {
        let name =
            || natord::compare_ignore_case(&a.name, &b.name).then_with(|| a.path.cmp(&b.path));
        let cmp = match order {
            SortOrder::Name => name(),
            SortOrder::Modified => b.modified.cmp(&a.modified).then_with(name),
            SortOrder::Size => b.size.cmp(&a.size).then_with(name),
        };
        if reverse { cmp.reverse() } else { cmp }
    });
}
pub fn next_index(current: usize, len: usize, delta: isize, wrap: bool) -> Option<usize> {
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
pub fn valid_filename(name: &str) -> bool {
    !name.trim().is_empty()
        && !matches!(name, "." | "..")
        && !name
            .chars()
            .any(|c| c.is_control() || "/\\<>:\"|?*".contains(c))
        && !name.ends_with(['.', ' '])
        && {
            let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
            !matches!(
                stem.as_str(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            )
        }
}
pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    } else {
        format!("{:.2} GB", bytes as f64 / 1073741824.0)
    }
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
    #[test]
    fn filename_validation_prevents_traversal_and_windows_reserved_names() {
        for s in [
            "../a.png", "a/b.png", "CON.png", "..", "", "a.png.", "LPT1.jpg",
        ] {
            assert!(!valid_filename(s), "accepted {s}");
        }
        assert!(valid_filename("summer photo 01.png"));
    }
    #[test]
    fn folder_scan_is_natural_and_ignores_unrelated_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for name in ["image10.PNG", "image2.png", "image1.jpg", "notes.txt"] {
            std::fs::write(dir.path().join(name), b"fixture")?;
        }
        let mut files = scan(dir.path())?;
        sort(&mut files, SortOrder::Name, false);
        assert_eq!(
            files.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["image1.jpg", "image2.png", "image10.PNG"]
        );
        Ok(())
    }
}
