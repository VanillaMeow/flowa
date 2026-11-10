//! The frame clock of animated images, independent of UI repaint scheduling

use std::time::{Duration, Instant};

use super::image_session::ImageFrame;

pub struct Animation {
    pub frame_index: usize,
    pub playing: bool,
    deadline: Instant,
}

impl Animation {
    pub fn new() -> Self {
        Self {
            frame_index: 0,
            playing: true,
            deadline: Instant::now(),
        }
    }

    pub fn start(&mut self, frames: &[ImageFrame], now: Instant) {
        self.frame_index = 0;
        self.playing = true;
        self.deadline =
            now + frames.first().map_or(Duration::ZERO, |frame| frame.delay);
    }

    pub fn toggle(&mut self, now: Instant) {
        self.playing = !self.playing;
        self.deadline = now;
    }

    pub fn advance(&mut self, frames: &[ImageFrame], now: Instant) -> Option<Duration> {
        if frames.len() <= 1 || !self.playing {
            return None;
        }

        if now >= self.deadline {
            self.frame_index = (self.frame_index + 1) % frames.len();
            self.deadline = now + frames[self.frame_index].delay;
        }

        Some(self.deadline.saturating_duration_since(now))
    }
}
