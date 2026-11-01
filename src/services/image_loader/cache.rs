//! A small CPU cache shared by selected and speculative loads

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::viewer::image_session::DecodedImage;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct FileStamp {
    size: u64,
    modified: SystemTime,
}

impl FileStamp {
    pub fn read(path: &Path) -> Option<Self> {
        let metadata = path.metadata().ok()?;

        Some(Self {
            size: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }
}

struct CachedImage {
    path: PathBuf,
    stamp: FileStamp,
    image: DecodedImage,
}

#[derive(Default)]
pub(super) struct ImageCache {
    entries: VecDeque<CachedImage>,
    pub budget: u64,
}

impl ImageCache {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn set_budget(&mut self, image_budget: u64) {
        self.budget = (image_budget / 16).min(64 * 1024 * 1024);
        self.make_room(0);
    }

    pub fn contains(&self, path: &Path, stamp: Option<FileStamp>) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.path == path && Some(entry.stamp) == stamp)
    }

    pub fn take(
        &mut self,
        path: &Path,
        stamp: Option<FileStamp>,
    ) -> Option<DecodedImage> {
        let index = self.entries.iter().position(|entry| entry.path == path)?;
        let entry = self.entries.remove(index)?;

        (Some(entry.stamp) == stamp).then_some(entry.image)
    }

    pub fn make_room(&mut self, reserved: u64) {
        while self.used_bytes().saturating_add(reserved) > self.budget {
            if self.entries.pop_front().is_none() {
                break;
            }
        }
    }

    pub fn insert(&mut self, path: PathBuf, stamp: FileStamp, image: DecodedImage) {
        let bytes = image.memory_bytes();

        if image.frames.len() != 1 || bytes > self.budget {
            return;
        }

        self.entries.retain(|entry| entry.path != path);
        self.make_room(bytes);

        if self.entries.len() == 4 {
            self.entries.pop_front();
        }

        self.entries.push_back(CachedImage { path, stamp, image });
    }

    fn used_bytes(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| entry.image.memory_bytes())
            .sum()
    }
}
