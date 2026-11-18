//! Decoded frames around the shown one, so stepping back and then forward again needs
//! no decoding
//!
//! Frames shown during playback or stepping are remembered, and frames decoded on the
//! way to a backward step's seek target are kept too. Hardware decoders get extra pool
//! surfaces for the frames held here, and software frames are bounded by memory

use std::collections::VecDeque;

use crate::video::decoding::{DecodedFrame, RETAINED_HARDWARE_FRAMES};

/// Memory software frames may hold for stepping back
const SOFTWARE_BUDGET: usize = 256 * 1024 * 1024;
const MIN_SOFTWARE_FRAMES: usize = 4;
const MAX_SOFTWARE_FRAMES: usize = 48;

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
}

impl FrameHistory {
    pub fn clear(&mut self) {
        self.behind.clear();
        self.ahead.clear();
    }

    /// Keeps a frame that was shown or decoded before the shown one, dropping the
    /// oldest beyond the limit
    pub fn remember(&mut self, frame: TimedFrame) {
        let limit = limit(&frame.frame);
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
}

/// How many frames like this one may be held
fn limit(frame: &DecodedFrame) -> usize {
    match frame {
        DecodedFrame::Hardware(_) => RETAINED_HARDWARE_FRAMES,
        DecodedFrame::Cpu(_) => {
            let info = frame.info();
            let pixels = (info.width.max(1) as usize) * (info.height.max(1) as usize);

            // Up to two bytes per pixel covers 4:2:0 video at ten or more bits
            (SOFTWARE_BUDGET / (pixels * 2))
                .clamp(MIN_SOFTWARE_FRAMES, MAX_SOFTWARE_FRAMES)
        }
    }
}
