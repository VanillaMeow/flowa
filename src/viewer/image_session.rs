//! Selected image loading and animation playback

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use egui::ColorImage;
use image::RgbaImage;

use super::playback::Animation;

pub struct DecodedFrame {
    pub pixels: Arc<RgbaImage>,
    pub preview: ColorImage,
    pub delay: Duration,
}

pub struct DecodedImage {
    pub frames: Vec<DecodedFrame>,
    pub thumbnail: ColorImage,
    pub size: [u32; 2],
    pub format: String,
    pub file_size: u64,
    pub color: String,
    pub metadata: Vec<(String, String)>,
}

impl DecodedImage {
    pub fn duration(&self) -> Duration {
        self.frames.iter().map(|f| f.delay).sum()
    }
}

pub(crate) struct ImageSession {
    generation: u64,
    pub decoded: Option<DecodedImage>,
    pub loading: bool,
    pub error: Option<String>,
    pub playback: Animation,
}

impl ImageSession {
    pub fn new() -> Self {
        Self {
            generation: 0,
            decoded: None,
            loading: false,
            error: None,
            playback: Animation::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn begin_scan(&mut self) {
        self.loading = true;
        self.error = None;
    }

    pub fn begin_load(&mut self) -> u64 {
        self.clear();
        self.loading = true;

        self.generation
    }

    pub fn clear(&mut self) {
        // Advancing the generation prevents queued results from restoring an old image
        self.generation += 1;
        self.decoded = None;
        self.loading = false;
        self.error = None;
        self.playback.frame_index = 0;
    }

    pub fn accept(&mut self, generation: u64, result: Result<DecodedImage>) -> bool {
        if generation != self.generation {
            return false;
        }

        self.loading = false;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded() -> DecodedImage {
        DecodedImage {
            frames: vec![DecodedFrame {
                pixels: Arc::new(image::RgbaImage::new(1, 1)),
                preview: egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
                delay: Duration::from_millis(100),
            }],
            thumbnail: egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
            size: [1, 1],
            format: "PNG".into(),
            file_size: 0,
            color: "RGBA".into(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn clearing_an_image_rejects_its_completed_decode() {
        let mut session = ImageSession::new();
        let generation = session.begin_load();
        session.clear();

        assert!(!session.accept(generation, Ok(decoded())));
        assert!(session.decoded.is_none());
        assert!(!session.loading);
    }

    #[test]
    fn stale_failure_does_not_interrupt_a_new_selection() {
        let mut session = ImageSession::new();
        let old = session.begin_load();
        let current = session.begin_load();

        assert!(!session.accept(old, Err(anyhow::anyhow!("Old image failed"))));
        assert!(session.loading);
        assert!(session.error.is_none());
        assert!(session.accept(current, Ok(decoded())));
        assert!(session.current_pixels().is_some());
        assert!(!session.loading);
    }
}
