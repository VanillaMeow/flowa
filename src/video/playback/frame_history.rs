//! Decoded frames around the shown one, so stepping back and then forward again needs
//! no decoding
//!
//! Frames shown during playback or stepping are remembered, and batches from the
//! background decoder are added on either side. Frames are bounded by count and
//! memory, with more allowed while the user steps frame by frame

use std::collections::VecDeque;

use crate::video::decoding::{DecodedFrame, kept_frames};

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

    /// Allows more frames while stepping; playing again drops the oldest extra frames
    /// as new ones are remembered
    pub fn set_stepping(&mut self, stepping: bool) {
        self.stepping = stepping;
    }

    /// How many frames like this one may be held
    pub fn limit(&self, frame: &DecodedFrame) -> usize {
        let info = frame.info();
        kept_frames(info.width, info.height, self.stepping)
    }

    pub fn len(&self) -> usize {
        self.behind.len() + self.ahead.len()
    }

    pub fn behind_len(&self) -> usize {
        self.behind.len()
    }

    pub fn ahead_len(&self) -> usize {
        self.ahead.len()
    }

    /// Where the oldest kept frame sits
    pub fn oldest(&self) -> Option<f64> {
        self.behind.front().map(|frame| frame.position)
    }

    /// Keeps a frame that was shown or decoded before the shown one, dropping the
    /// oldest beyond the limit
    pub fn remember(&mut self, frame: TimedFrame) {
        let limit = self.limit(&frame.frame);
        self.behind.push_back(frame);
        self.drop_oldest(limit);
    }

    /// The frame shown just before the current one
    pub fn previous(&mut self) -> Option<TimedFrame> {
        self.behind.pop_back()
    }

    /// Puts a frame back in front of those already decoded ahead
    pub fn push_ahead(&mut self, frame: TimedFrame) {
        self.ahead.push_front(frame);
    }

    /// The next frame already decoded, ahead of anything the decoder has
    pub fn next(&mut self) -> Option<TimedFrame> {
        self.ahead.pop_front()
    }

    /// Drops frames ahead beyond the nearest `keep`, making room for earlier ones;
    /// returns where the first dropped frame was
    pub fn trim_ahead(&mut self, keep: usize) -> Option<f64> {
        let dropped = self.ahead.get(keep).map(|frame| frame.position);
        self.ahead.truncate(keep);
        dropped
    }

    /// Drops the oldest frames until at most `limit` are kept
    pub fn drop_oldest(&mut self, limit: usize) {
        while self.len() > limit && self.behind.pop_front().is_some() {}
    }

    /// Puts earlier frames, oldest first, before every frame kept behind
    pub fn prepend(&mut self, frames: Vec<TimedFrame>) {
        for frame in frames.into_iter().rev() {
            self.behind.push_front(frame);
        }
    }

    /// Puts later frames, in order, after every frame kept ahead
    pub fn append(&mut self, frames: Vec<TimedFrame>) {
        self.ahead.extend(frames);
    }
}
