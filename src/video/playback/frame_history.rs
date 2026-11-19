//! Decoded frames around the shown one, so stepping back and then forward again needs
//! no decoding
//!
//! Frames shown during playback or stepping are remembered, and earlier frames decoded
//! in the background while stepping back are added in front. Frames are bounded by
//! memory, with more allowed while the user steps frame by frame, so batches of earlier
//! frames can be larger. VA-API frames are bounded by the decoder's fixed surface pool

use std::collections::VecDeque;

use crate::video::decoding::{DecodedFrame, HardwareStorage, retained_frames};

/// Memory held frames may take during playback
const PLAYING_BUDGET: usize = 256 * 1024 * 1024;
const PLAYING_FRAMES: std::ops::RangeInclusive<usize> = 4..=32;
/// Memory held frames may take while stepping frame by frame
const STEPPING_BUDGET: usize = 512 * 1024 * 1024;
const STEPPING_FRAMES: std::ops::RangeInclusive<usize> = 8..=64;

pub(super) struct TimedFrame {
    pub frame: DecodedFrame,
    pub position: f64,
}

#[derive(Default)]
pub(super) struct FrameHistory {
    /// Frames before the shown one, oldest first
    behind: VecDeque<TimedFrame>,
    /// Frames after the engine's pending frame, next first
    ahead: VecDeque<TimedFrame>,
    /// Whether the user is stepping frame by frame, which allows more frames
    stepping: bool,
}

impl FrameHistory {
    pub fn clear(&mut self) {
        self.behind.clear();
        self.ahead.clear();
    }

    /// Keeps a frame that was shown or decoded before the shown one, dropping the
    /// oldest beyond the limit
    pub fn remember(&mut self, frame: TimedFrame) {
        let limit = self.limit(&frame.frame);
        self.behind.push_back(frame);

        while self.behind.len() + self.ahead.len() > limit {
            self.behind.pop_front();
        }
    }

    /// The frame shown just before the current one
    pub fn previous(&mut self) -> Option<TimedFrame> {
        self.behind.pop_back()
    }

    pub fn has_previous(&self) -> bool {
        !self.behind.is_empty()
    }

    /// Puts a frame back in front of those already decoded ahead
    pub fn push_ahead(&mut self, frame: TimedFrame) {
        self.ahead.push_front(frame);
    }

    /// The next frame already decoded, ahead of anything the decoder has
    pub fn next(&mut self) -> Option<TimedFrame> {
        self.ahead.pop_front()
    }

    pub fn len(&self) -> usize {
        self.behind.len() + self.ahead.len()
    }

    pub fn behind_len(&self) -> usize {
        self.behind.len()
    }

    /// Where the oldest kept frame sits
    pub fn oldest(&self) -> Option<f64> {
        self.behind.front().map(|frame| frame.position)
    }

    /// Drops frames ahead beyond the nearest `keep`, making room for earlier ones;
    /// returns where the first dropped frame was, which the decoder must reach again
    pub fn trim_ahead(&mut self, keep: usize) -> Option<f64> {
        if self.ahead.len() <= keep {
            return None;
        }

        let dropped = self.ahead.get(keep).map(|frame| frame.position);
        self.ahead.truncate(keep);

        dropped
    }

    /// Puts earlier frames, oldest first, before every frame kept behind
    pub fn prepend(&mut self, frames: Vec<TimedFrame>) {
        for frame in frames.into_iter().rev() {
            self.behind.push_front(frame);
        }
    }
}

impl FrameHistory {
    /// Allows more frames while stepping; playing again drops the oldest extra frames
    /// as new ones are remembered
    pub fn set_stepping(&mut self, stepping: bool) {
        self.stepping = stepping;
    }

    /// How many frames like this one may be held
    pub fn limit(&self, frame: &DecodedFrame) -> usize {
        let info = frame.info();

        if let DecodedFrame::Hardware(hardware) = frame
            && hardware.storage() == HardwareStorage::Vaapi
        {
            return retained_frames(info.width, info.height);
        }

        let (budget, range) = if self.stepping {
            (STEPPING_BUDGET, STEPPING_FRAMES)
        } else {
            (PLAYING_BUDGET, PLAYING_FRAMES)
        };
        let pixels = (info.width.max(1) as usize) * (info.height.max(1) as usize);

        // Up to two bytes per pixel covers 4:2:0 video at ten or more bits
        (budget / (pixels * 2)).clamp(*range.start(), *range.end())
    }
}
