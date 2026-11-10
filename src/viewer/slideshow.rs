//! The slideshow clock, independent of UI repaint scheduling

use std::time::{Duration, Instant};

pub struct Slideshow {
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
