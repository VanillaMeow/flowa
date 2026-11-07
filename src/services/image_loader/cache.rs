//! A small CPU cache shared by selected and speculative loads

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::viewer::image_session::DecodedImage;

const MAX_CACHED_IMAGES: usize = 4;
const MAX_CACHE_BYTES: u64 = 512 * 1024 * 1024;

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

    /// Allows a quarter of the image memory limit, enough for a few camera photos
    pub fn set_budget(&mut self, image_budget: u64) {
        self.budget = (image_budget / 4).min(MAX_CACHE_BYTES);
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

        if self.entries.len() == MAX_CACHED_IMAGES {
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

#[cfg(test)]
mod tests {
    use super::{FileStamp, ImageCache};
    use crate::services::image_decode::decode;
    use anyhow::{Context, Result};
    use std::{fs::File, sync::Arc, time::Duration};

    const IMAGE_BUDGET: u64 = 1024 * 1024;

    #[test]
    fn cache_reuses_pixels_rejects_modified_files_and_reserves_working_memory()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("image.png");
        image::RgbaImage::new(32, 16).save(&path)?;
        let stamp = FileStamp::read(&path).context("Missing file timestamp")?;
        let mut cache = ImageCache::default();
        cache.set_budget(IMAGE_BUDGET);
        let decoded = decode(&path, IMAGE_BUDGET)?;
        let pixels = Arc::clone(&decoded.frames[0].pixels);
        cache.insert(path.clone(), stamp, decoded);

        let cached = cache
            .take(&path, Some(stamp))
            .context("Missing cached image")?;
        assert!(Arc::ptr_eq(&pixels, &cached.frames[0].pixels));
        cache.insert(path.clone(), stamp, cached);

        let modified = path.metadata()?.modified()? + Duration::from_secs(2);
        File::options()
            .write(true)
            .open(&path)?
            .set_modified(modified)?;
        assert!(cache.take(&path, FileStamp::read(&path)).is_none());

        let stamp = FileStamp::read(&path).context("Missing updated timestamp")?;
        cache.insert(path.clone(), stamp, decode(&path, IMAGE_BUDGET)?);
        cache.make_room(cache.budget);
        assert!(cache.take(&path, Some(stamp)).is_none());

        Ok(())
    }
}
