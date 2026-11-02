//! Selected loads take priority over a bounded queue of speculative decodes

mod cache;

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Context, Result};

use crate::viewer::image_session::DecodedImage;

use super::image_decode::{decode, preload_bytes};
use cache::{FileStamp, ImageCache};

pub struct LoadRequest {
    pub generation: u64,
    pub path: PathBuf,
    pub budget: u64,
}

pub struct LoadResult {
    pub generation: u64,
    pub result: Result<DecodedImage>,
}

enum Message {
    Select(LoadRequest),
    Hover(PathBuf),
    Neighbors(Vec<PathBuf>),
    Budget(u64),
    Invalidate,
    Cancel,
}

pub struct Loader {
    sender: mpsc::Sender<Message>,
    pub results: mpsc::Receiver<LoadResult>,
    generation: Arc<AtomicU64>,
}

impl Loader {
    pub fn new(ctx: egui::Context) -> Self {
        let (sender, requests) = mpsc::channel();
        let (completed, results) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let active_generation = Arc::clone(&generation);

        thread::spawn(move || run(requests, completed, active_generation, ctx));

        Self {
            sender,
            results,
            generation,
        }
    }

    pub fn request(&self, request: LoadRequest) {
        self.generation.store(request.generation, Ordering::Relaxed);
        let _ = self.sender.send(Message::Select(request));
    }

    pub fn preload(&self, path: PathBuf) {
        let _ = self.sender.send(Message::Hover(path));
    }

    pub fn preload_neighbors(&self, paths: Vec<PathBuf>) {
        let _ = self.sender.send(Message::Neighbors(paths));
    }

    pub fn set_budget(&self, budget: u64) {
        let _ = self.sender.send(Message::Budget(budget));
    }

    pub fn invalidate(&self) {
        let _ = self.sender.send(Message::Invalidate);
    }

    pub fn cancel(&self, generation: u64) {
        self.generation.store(generation, Ordering::Relaxed);
        let _ = self.sender.send(Message::Cancel);
    }
}

struct DecodeJob {
    path: PathBuf,
    budget: u64,
    generation: Option<u64>,
}

#[derive(Default)]
struct Queue {
    selected: Option<LoadRequest>,
    preloads: VecDeque<PathBuf>,
    skipped: VecDeque<(PathBuf, Option<FileStamp>)>,
    cache: ImageCache,
    revision: u64,
}

impl Queue {
    fn receive(&mut self, message: Message) {
        match message {
            Message::Select(request) => {
                self.cache.set_budget(request.budget);
                self.selected = Some(request);
                self.preloads.clear();
            }
            Message::Hover(path) => {
                self.preloads.retain(|pending| *pending != path);
                self.preloads.push_front(path);
                self.preloads.truncate(3);
            }
            Message::Neighbors(paths) => {
                self.preloads = paths.into_iter().take(2).collect()
            }
            Message::Budget(budget) => {
                self.cache.set_budget(budget);
                self.preloads.clear();
                self.skipped.clear();
                self.revision += 1;

                if let Some(selected) = &mut self.selected {
                    selected.budget = budget;
                }
            }
            Message::Invalidate | Message::Cancel => {
                self.cache.clear();
                self.preloads.clear();
                self.skipped.clear();
                self.revision += 1;

                if matches!(message, Message::Cancel) {
                    self.selected = None;
                }
            }
        }
    }

    fn drain(&mut self, requests: &mpsc::Receiver<Message>) {
        while let Ok(message) = requests.try_recv() {
            self.receive(message);
        }
    }

    fn next(&mut self) -> Option<DecodeJob> {
        if let Some(request) = self.selected.take() {
            return Some(DecodeJob {
                path: request.path,
                budget: request.budget,
                generation: Some(request.generation),
            });
        }

        Some(DecodeJob {
            generation: None,
            path: self.preloads.pop_front()?,
            budget: self.cache.budget,
        })
    }

    fn load(
        &mut self,
        request: &DecodeJob,
        stamp: Option<FileStamp>,
    ) -> Option<Result<DecodedImage>> {
        if request.generation.is_some() {
            if let Some(image) = self.cache.take(&request.path, stamp) {
                log::debug!("Using preloaded image {}", request.path.display());
                return Some(Ok(image));
            }

            return Some(decode(&request.path, request.budget));
        }

        if self.cache.contains(&request.path, stamp)
            || self
                .skipped
                .iter()
                .any(|(path, previous)| *path == request.path && *previous == stamp)
        {
            return None;
        }

        let reserved = preload_bytes(&request.path, request.budget).ok().flatten();
        let Some(reserved) = reserved else {
            self.skip(request.path.clone(), stamp);
            return None;
        };

        self.cache.make_room(reserved);
        log::debug!("Preloading {}", request.path.display());
        let result = decode(&request.path, request.budget);

        if result.is_err() {
            self.skip(request.path.clone(), stamp);
        }

        Some(result)
    }

    fn skip(&mut self, path: PathBuf, stamp: Option<FileStamp>) {
        if self.skipped.len() == 32 {
            self.skipped.pop_front();
        }

        self.skipped.push_back((path, stamp));
    }

    fn finish(
        &mut self,
        request: DecodeJob,
        stamp: Option<FileStamp>,
        revision: u64,
        result: Result<DecodedImage>,
        active_generation: u64,
    ) -> Option<LoadResult> {
        let unchanged = stamp.is_some() && stamp == FileStamp::read(&request.path);
        let reusable = unchanged && revision == self.revision;

        if reusable && self.can_promote(&request, &result, active_generation) {
            let selected = self.selected.take()?;
            log::debug!("Adopting preload {}", request.path.display());

            return Some(LoadResult {
                generation: selected.generation,
                result,
            });
        }

        if request.generation == Some(active_generation) {
            return Some(LoadResult {
                generation: active_generation,
                result,
            });
        }

        if reusable && let (Some(stamp), Ok(image)) = (stamp, result) {
            self.cache.insert(request.path, stamp, image);
        }

        None
    }

    fn can_promote(
        &self,
        request: &DecodeJob,
        result: &Result<DecodedImage>,
        generation: u64,
    ) -> bool {
        let Some(selected) = &self.selected else {
            return false;
        };

        if selected.path != request.path || selected.generation != generation {
            return false;
        }

        result
            .as_ref()
            .is_ok_and(|image| image.memory_bytes() <= selected.budget)
    }
}

fn run(
    requests: mpsc::Receiver<Message>,
    completed: mpsc::SyncSender<LoadResult>,
    generation: Arc<AtomicU64>,
    ctx: egui::Context,
) {
    let mut queue = Queue::default();

    loop {
        if queue.selected.is_none() && queue.preloads.is_empty() {
            let Ok(message) = requests.recv() else { return };
            queue.receive(message);
        }

        queue.drain(&requests);
        let Some(request) = queue.next() else {
            continue;
        };

        if request
            .generation
            .is_some_and(|requested| requested != generation.load(Ordering::Relaxed))
        {
            continue;
        }

        let stamp = FileStamp::read(&request.path);
        let revision = queue.revision;
        let Some(result) = queue.load(&request, stamp) else {
            continue;
        };
        let result = result
            .with_context(|| format!("Could not open {}", request.path.display()));

        // A click during preloading adopts the completed decode without starting it again
        queue.drain(&requests);
        let result = queue.finish(
            request,
            stamp,
            revision,
            result,
            generation.load(Ordering::Relaxed),
        );

        if let Some(result) = result {
            if completed.send(result).is_err() {
                return;
            }

            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use anyhow::{Context, Result};

    use super::{FileStamp, LoadRequest, Message, Queue};

    const IMAGE_BUDGET: u64 = 1024 * 1024;

    fn save_image(path: &Path) -> Result<()> {
        image::RgbaImage::new(32, 16).save(path)?;

        Ok(())
    }

    #[test]
    fn queue_keeps_latest_selection_and_prioritizes_hover() -> Result<()> {
        let mut queue = Queue::default();
        queue.receive(Message::Neighbors(vec![
            "next.png".into(),
            "previous.png".into(),
        ]));
        queue.receive(Message::Hover("hover.png".into()));
        assert_eq!(
            queue.next().context("Missing hover")?.path,
            Path::new("hover.png")
        );

        for generation in 1..=3 {
            queue.receive(Message::Select(LoadRequest {
                generation,
                path: "selected.png".into(),
                budget: IMAGE_BUDGET,
            }));
        }

        let selected = queue.next().context("Missing selection")?;
        assert_eq!(selected.generation, Some(3));

        Ok(())
    }

    #[test]
    fn click_during_preload_adopts_the_same_pixels() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("image.png");
        save_image(&path)?;
        let mut queue = Queue::default();
        queue.receive(Message::Budget(IMAGE_BUDGET));
        queue.receive(Message::Hover(path.clone()));
        let job = queue.next().context("Missing preload")?;
        let revision = queue.revision;
        let stamp = FileStamp::read(&path);
        let decoded = queue.load(&job, stamp).context("Preload skipped")??;
        let pixels = Arc::clone(&decoded.frames[0].pixels);

        queue.receive(Message::Select(LoadRequest {
            generation: 2,
            path: path.clone(),
            budget: IMAGE_BUDGET,
        }));
        let completed = queue.finish(job, stamp, revision, Ok(decoded), 2);
        let completed = completed.context("Preload was not promoted")?;
        assert_eq!(completed.generation, 2);
        let decoded = completed.result?;
        assert_eq!(decoded.size, [32, 16]);
        assert!(Arc::ptr_eq(&pixels, &decoded.frames[0].pixels));

        Ok(())
    }

    #[test]
    fn preloading_skips_images_that_exceed_the_budget() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("image.png");
        save_image(&path)?;
        let mut queue = Queue::default();
        queue.receive(Message::Budget(IMAGE_BUDGET));
        queue.receive(Message::Hover(path.clone()));
        let job = queue.next().context("Missing preload")?;
        queue
            .load(&job, FileStamp::read(&path))
            .context("Preload skipped")??;

        queue.receive(Message::Budget(1024));
        queue.receive(Message::Hover(path.clone()));
        let job = queue.next().context("Missing preload")?;
        assert!(queue.load(&job, FileStamp::read(&path)).is_none());

        Ok(())
    }

    #[test]
    fn invalidation_discards_in_flight_preloads_and_preserves_the_click() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("image.png");
        save_image(&path)?;
        let mut queue = Queue::default();
        queue.receive(Message::Budget(IMAGE_BUDGET));
        queue.receive(Message::Hover(path.clone()));
        let job = queue.next().context("Missing preload")?;
        let revision = queue.revision;
        let stamp = FileStamp::read(&path);
        let result = queue.load(&job, stamp).context("Preload skipped")?;

        queue.receive(Message::Invalidate);
        queue.receive(Message::Select(LoadRequest {
            generation: 2,
            path: path.clone(),
            budget: IMAGE_BUDGET,
        }));
        assert!(queue.finish(job, stamp, revision, result, 2).is_none());
        assert!(!queue.cache.contains(&path, stamp));
        assert_eq!(queue.next().context("Click was lost")?.generation, Some(2));

        Ok(())
    }
}
