//! Animation and slideshow clocks independent of UI repaint scheduling

use std::time::{Duration, Instant};

use super::image_session::DecodedFrame;

pub(crate) struct Animation {
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

    pub fn start(&mut self, frames: &[DecodedFrame], now: Instant) {
        self.frame_index = 0;
        self.playing = true;
        self.deadline =
            now + frames.first().map_or(Duration::ZERO, |frame| frame.delay);
    }

    pub fn toggle(&mut self, now: Instant) {
        self.playing = !self.playing;
        self.deadline = now;
    }

    pub fn advance(
        &mut self,
        frames: &[DecodedFrame],
        now: Instant,
    ) -> Option<Duration> {
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

pub(crate) struct Slideshow {
    pub active: bool,
    next_slide: Instant,
}

impl Slideshow {
    pub fn new() -> Self {
        Self {
            active: false,
            next_slide: Instant::now(),
        }
    }

    pub fn restart(&mut self, seconds: f32) {
        self.next_slide = Instant::now() + Duration::from_secs_f32(seconds);
    }

    pub fn toggle(&mut self, seconds: f32) {
        self.active = !self.active;
        self.restart(seconds);
    }

    pub fn remaining(&self, now: Instant, ready: bool) -> Option<Duration> {
        if self.active && ready {
            Some(self.next_slide.saturating_duration_since(now))
        } else {
            None
        }
    }
}
