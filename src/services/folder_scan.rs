//! Filesystem metadata and background folder scans

use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::SystemTime,
};

use anyhow::{Context, Result, ensure};

use crate::viewer::catalog::{Entry, ScannedFolder, supported};

struct ScanResult {
    generation: u64,
    result: Result<ScannedFolder>,
}

pub(crate) struct FolderScanner {
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

        std::thread::spawn(move || {
            let result = scan_path(path);
            let _ = sender.send(ScanResult { generation, result });

            ctx.request_repaint();
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

        if supported(&path)
            && let Ok(entry) = entry_from_path(path)
        {
            files.push(entry);
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use crate::{settings::SortOrder, viewer::catalog};

    use super::*;

    #[test]
    fn folder_scan_is_natural_and_ignores_unrelated_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for name in ["image10.PNG", "image2.png", "image1.jpg", "notes.txt"] {
            std::fs::write(dir.path().join(name), b"fixture")?;
        }
        let mut files = scan(dir.path())?;
        catalog::sort(&mut files, SortOrder::Name, false);
        assert_eq!(
            files.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["image1.jpg", "image2.png", "image10.PNG"]
        );
        Ok(())
    }

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
