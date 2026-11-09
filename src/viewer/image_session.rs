//! Selected image loading and animation playback

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use egui::ColorImage;
use image::RgbaImage;

use super::{playback::Animation, vector::VectorImage};

pub struct DecodedFrame {
    pub pixels: Arc<RgbaImage>,
    pub preview: ColorImage,
    pub delay: Duration,
}

pub struct DecodedImage {
    pub frames: Vec<DecodedFrame>,
    /// The source of vector formats, which zoomed views render from instead of pixels
    pub vector: Option<Arc<VectorImage>>,
    pub thumbnail: ColorImage,
    pub size: [u32; 2],
    pub format: String,
    pub file_size: u64,
    pub color: String,
    pub metadata: Vec<(String, String)>,
}

impl DecodedImage {
    pub fn memory_bytes(&self) -> u64 {
        let frames: usize = self
            .frames
            .iter()
            .map(|frame| frame.pixels.as_raw().len() + frame.preview.pixels.len() * 4)
            .sum();

        frames as u64 + self.thumbnail.pixels.len() as u64 * 4
    }

    pub fn duration(&self) -> Duration {
        self.frames.iter().map(|f| f.delay).sum()
    }
}

pub(crate) struct ImageSession {
    generation: u64,
    pub decoded: Option<DecodedImage>,
    pub loading: bool,
    loading_started: Option<Instant>,
    pub error: Option<String>,
    pub playback: Animation,
}

impl ImageSession {
    pub fn new() -> Self {
        Self {
            generation: 0,
            decoded: None,
            loading: false,
            loading_started: None,
            error: None,
            playback: Animation::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn begin_load(&mut self) -> u64 {
        self.clear();
        self.loading = true;
        self.loading_started = Some(Instant::now());

        self.generation
    }

    pub fn clear(&mut self) {
        // Advancing the generation prevents queued results from restoring an old image
        self.generation += 1;
        self.decoded = None;
        self.loading = false;
        self.loading_started = None;
        self.error = None;
        self.playback.frame_index = 0;
    }

    pub fn accept(&mut self, generation: u64, result: Result<DecodedImage>) -> bool {
        if generation != self.generation {
            return false;
        }

        self.loading = false;
        self.loading_started = None;

        match result {
            Ok(decoded) => {
                self.playback.start(&decoded.frames, Instant::now());
                self.decoded = Some(decoded);
                self.error = None;
            }
            Err(error) => {
                self.decoded = None;
                self.error = Some(format!("{error:#}"));
            }
        }

        true
    }

    pub fn loading_age(&self) -> Duration {
        self.loading_started
            .map(|started| started.elapsed())
            .unwrap_or_default()
    }

    pub fn current_pixels(&self) -> Option<Arc<image::RgbaImage>> {
        self.decoded
            .as_ref()?
            .frames
            .get(self.playback.frame_index)
            .map(|frame| Arc::clone(&frame.pixels))
    }

    pub fn animated(&self) -> bool {
        self.decoded
            .as_ref()
            .is_some_and(|image| image.frames.len() > 1)
    }

    pub fn toggle_play(&mut self) {
        self.playback.toggle(Instant::now());
    }

    pub fn animate(&mut self, now: Instant) -> Option<Duration> {
        self.playback.advance(&self.decoded.as_ref()?.frames, now)
    }
}
