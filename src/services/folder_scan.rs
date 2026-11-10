//! Filesystem metadata and background folder scans

use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::SystemTime,
};

use anyhow::{Context, Result, ensure};

use super::worker;
use crate::viewer::{
    catalog::{Entry, ScannedFolder},
    formats::{self, MediaKind},
};

struct ScanResult {
    generation: u64,
    result: Result<ScannedFolder>,
}

pub struct FolderScanner {
    sender: mpsc::Sender<ScanResult>,
    results: mpsc::Receiver<ScanResult>,
}

impl FolderScanner {
    pub fn new() -> Self {
        let (sender, results) = mpsc::channel();

        Self { sender, results }
    }

    pub fn request(&self, path: PathBuf, generation: u64, ctx: &egui::Context) {
        let sender = self.sender.clone();
        let ctx = ctx.clone();

        // Each scan gets its own thread, so a slow network folder never delays a newer one
        worker::spawn("folder-scan", move || {
            let result = scan_path(path);

            if sender.send(ScanResult { generation, result }).is_ok() {
                ctx.request_repaint();
            }
        });
    }

    pub fn poll(&self, generation: u64) -> Option<Result<ScannedFolder>> {
        while let Ok(result) = self.results.try_recv() {
            // Folder scans can finish after a newer folder has already opened
            if result.generation != generation {
                continue;
            }

            return Some(result.result);
        }

        None
    }
}

fn scan_path(path: PathBuf) -> Result<ScannedFolder> {
    let path = path.canonicalize().unwrap_or(path);

    let selected = if path.is_dir() {
        None
    } else {
        Some(path.clone())
    };
    let folder = if path.is_dir() {
        path.clone()
    } else {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };

    let mut entries = scan(&folder)?;

    if let Some(selected) = &selected
        && !entries.iter().any(|entry| entry.path == *selected)
    {
        // An explicitly opened file may have a nonstandard extension
        entries.push(entry_from_path(selected.clone())?);
    }

    Ok(ScannedFolder {
        folder,
        selected,
        entries,
    })
}

pub fn entry_from_path(path: PathBuf) -> Result<Entry> {
    let metadata = path
        .metadata()
        .with_context(|| format!("Cannot read {}", path.display()))?;

    ensure!(metadata.is_file(), "This path is not a file");

    Ok(Entry {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        // Anything unrecognized is offered to the image decoders
        kind: formats::kind(&path).unwrap_or(MediaKind::Raster),
        size: metadata.len(),
        modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        path,
    })
}

pub fn scan(folder: &Path) -> Result<Vec<Entry>> {
    let entries = std::fs::read_dir(folder)
        .with_context(|| format!("Cannot open {}", folder.display()))?;
    let mut files = Vec::new();

    for item in entries {
        let item = item.context("Cannot read folder entry")?;
        let path = item.path();

        if formats::supported(&path)
            && let Ok(entry) = entry_from_path(path)
        {
            files.push(entry);
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::{FolderScanner, ScanResult};
    use crate::viewer::catalog::ScannedFolder;
    use anyhow::Result;
    use std::path::PathBuf;

    #[test]
    fn stale_catalog_results_are_discarded() -> Result<()> {
        let scanner = FolderScanner::new();
        for (generation, folder) in [(2, "current"), (1, "stale")] {
            scanner.sender.send(ScanResult {
                generation,
                result: Ok(ScannedFolder {
                    folder: PathBuf::from(folder),
                    selected: None,
                    entries: Vec::new(),
                }),
            })?;
        }
        let result = scanner.poll(2).transpose()?;
        assert_eq!(
            result.map(|folder| folder.folder),
            Some(PathBuf::from("current"))
        );
        assert!(scanner.poll(2).is_none());
        Ok(())
    }
}
