//! Frame stepping: the frames kept around the shown one, and the background decoder
//! that adds more of them while the user steps
//!
//! Backward steps show kept frames. While paused, a second decoder on the same device
//! decodes the batch before the oldest kept frame, making room by dropping the frames
//! furthest ahead. The main decoder then no longer continues the frames ahead, so
//! forward steps past them take forward batches from the second decoder until playback
//! resumes, which seeks. If the second decoder cannot run, backward steps seek and
//! keep the frames decoded on the way

use std::{path::Path, time::Instant};

use anyhow::Result;

use super::{
    frame_history::{FrameHistory, TimedFrame},
    prefetch::{Batch, Direction, Prefetch},
};
use crate::video::{decoding::Decoder, ffmpeg::StreamInfo};

/// Frames kept ahead of the shown one when making room for earlier ones, for stepping
/// forward again without waiting
const KEEP_AHEAD: usize = 2;
/// Steps that may wait for frames, so a held key never runs far ahead
pub(super) const MAX_WAITING_STEPS: u32 = 8;

#[derive(Default)]
pub(super) struct Stepping {
    history: FrameHistory,
    /// The second decoder, open while paused
    prefetch: Option<Prefetch>,
    /// Set once the second decoder failed for this video
    failed: bool,
    /// The last step's direction since playback paused, forward when true
    forward: Option<bool>,
    /// Backward steps waiting for earlier frames
    waiting_back: u32,
    /// Where the first frame missing ahead sits, once frames ahead were dropped
    gap: Option<f64>,
    /// A batch that found nothing, as at either end of the video
    exhausted: Option<Direction>,
    /// Where the latest seeking backward step is headed, without the second decoder
    seek_target: Option<f64>,
    /// Set while that seek keeps the frames it decodes before its target
    backfill: bool,
}

/// What a slice of background work did
#[derive(Default)]
pub(super) struct Worked {
    /// Waiting backward steps that now show
    pub stepped_back: u32,
    /// A batch finished, so the next one can be planned
    pub finished: bool,
    /// Frames arrived ahead, which waiting forward steps can show
    pub added_ahead: bool,
}

impl Stepping {
    /// Forgets everything, as when a video closes
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Drops the second decoder and every kept frame, as when the decoder changes
    pub fn close(&mut self) {
        self.prefetch = None;
        self.forget_position();
        self.history.clear();
    }

    /// Forgets work tied to the shown position, as when seeking
    pub fn forget_position(&mut self) {
        if let Some(prefetch) = &mut self.prefetch {
            prefetch.cancel();
        }
        self.waiting_back = 0;
        self.gap = None;
        self.exhausted = None;
        self.seek_target = None;
        self.backfill = false;
    }

    /// Drops the kept frames, as when a seek lands elsewhere in the video
    pub fn clear_frames(&mut self) {
        self.history.clear();
    }

    /// Notes a step in one direction, which allows more kept frames
    pub fn begin(&mut self, forward: bool) {
        self.forward = Some(forward);
        self.history.set_stepping(true);
    }

    /// Ends stepping as playback resumes; returns whether frames ahead were dropped, in
    /// which case the caller seeks so the main decoder reads on from the shown frame
    pub fn resume(&mut self) -> bool {
        let gap = self.gap.is_some();
        self.prefetch = None;
        self.forward = None;
        self.history.set_stepping(false);
        self.forget_position();
        gap
    }

    /// Opens the second decoder unless it is open or failed for this video
    pub fn open(
        &mut self,
        path: &Path,
        info: StreamInfo,
        decoder: impl FnOnce() -> Result<Box<dyn Decoder>>,
    ) {
        if self.prefetch.is_some() || self.failed {
            return;
        }
        match decoder().and_then(|decoder| Prefetch::open(path, info, decoder)) {
            Ok(prefetch) => self.prefetch = Some(prefetch),
            Err(error) => self.fail(&error),
        }
    }

    pub fn busy(&self) -> bool {
        self.prefetch.as_ref().is_some_and(Prefetch::busy)
    }

    /// Where the first frame missing ahead sits; past it, the main decoder is out of
    /// step with the kept frames
    pub fn gap(&self) -> Option<f64> {
        self.gap
    }

    /// Whether forward steps are stuck at the gap because the second decoder failed
    pub fn stranded(&self) -> bool {
        self.gap.is_some() && self.prefetch.is_none()
    }

    pub fn remember(&mut self, frame: TimedFrame) {
        self.history.remember(frame);
    }

    pub fn next_ahead(&mut self) -> Option<TimedFrame> {
        self.history.next()
    }

    /// Swaps in up to `count` kept frames from before the shown one, keeping the shown
    /// and pending frames for stepping forward again; returns how many showed
    pub fn show_previous(
        &mut self,
        current: &mut Option<TimedFrame>,
        pending: &mut Option<TimedFrame>,
        count: u32,
    ) -> u32 {
        let mut shown = 0;
        while shown < count
            && let Some(previous) = self.history.previous()
        {
            if let Some(next) = pending.take() {
                self.history.push_ahead(next);
            }
            if let Some(shown_frame) = current.replace(previous) {
                self.history.push_ahead(shown_frame);
            }
            shown += 1;
        }
        shown
    }

    /// Lets backward steps beyond the kept frames wait for the second decoder; returns
    /// false when it is not open, so the caller seeks instead
    pub fn wait_back(&mut self, count: u32) -> bool {
        if self.prefetch.is_none() {
            return false;
        }
        self.waiting_back = (self.waiting_back + count).min(MAX_WAITING_STEPS);
        true
    }

    /// Stops waiting when no batch could start
    pub fn stop_waiting(&mut self) {
        self.waiting_back = 0;
    }

    /// Notes a backward step that seeks to `target` and keeps frames before it
    pub fn seeking_back(&mut self, target: f64) {
        self.seek_target = Some(target);
        self.backfill = true;
    }

    /// Where the latest seeking backward step is headed
    pub fn seek_target(&self) -> Option<f64> {
        self.seek_target
    }

    /// Whether frames decoded before a seek's target belong to the kept frames
    pub fn backfilling(&self) -> bool {
        self.backfill
    }

    /// Ends a seeking backward step once its frame shows
    pub fn seek_done(&mut self) {
        self.seek_target = None;
        self.backfill = false;
    }

    /// Starts the next batch the user's steps call for, if there is room for it
    pub fn plan(
        &mut self,
        current: &TimedFrame,
        frame_duration: f64,
        steps_forward: u32,
    ) {
        let Some(forward) = self.forward else {
            return;
        };
        if self.busy() || self.prefetch.is_none() {
            return;
        }
        let limit = self.history.limit(&current.frame);
        let wants_forward = self.waiting_back == 0 && (forward || steps_forward > 0);
        let planned = if wants_forward {
            self.plan_forward(limit)
        } else {
            self.plan_back(limit, current, frame_duration)
        };
        if let Err(error) = planned {
            self.fail(&error);
        }
    }

    /// Continues the frames ahead past the gap once few remain
    fn plan_forward(&mut self, limit: usize) -> Result<()> {
        let Some(from) = self.gap else {
            return Ok(());
        };
        let direction = Direction::Forward { from };
        if self.history.ahead_len() >= KEEP_AHEAD || self.exhausted == Some(direction) {
            return Ok(());
        }
        // The oldest frames make room, as stepping forward leaves them furthest behind
        let count = batch_size(limit);
        self.history.drop_oldest(limit.saturating_sub(count));
        self.start(direction, count)
    }

    /// Decodes the frames before the oldest kept one once there is room for enough of
    /// them, or sooner when few remain or steps are waiting
    fn plan_back(
        &mut self,
        limit: usize,
        current: &TimedFrame,
        frame_duration: f64,
    ) -> Result<()> {
        let before = self.history.oldest().unwrap_or(current.position);
        let direction = Direction::Back { before };
        // Nothing comes before the first frame
        if before < frame_duration * 0.5 || self.exhausted == Some(direction) {
            self.waiting_back = 0;
            return Ok(());
        }
        // A few frames are worth decoding once kept frames run low
        let low = self.history.behind_len() <= limit / 3 || self.waiting_back > 0;
        let minimum = if low { 1 } else { batch_size(limit) };
        // The frames furthest ahead make room, and the main decoder no longer follows
        if limit.saturating_sub(self.history.len()) < minimum
            && let Some(dropped) = self.history.trim_ahead(KEEP_AHEAD)
        {
            self.gap = Some(self.gap.map_or(dropped, |gap| gap.min(dropped)));
        }
        let room = limit.saturating_sub(self.history.len());
        if room < minimum {
            return Ok(());
        }
        self.start(direction, room)
    }

    fn start(&mut self, direction: Direction, count: usize) -> Result<()> {
        match &mut self.prefetch {
            Some(prefetch) => prefetch.start(direction, count),
            None => Ok(()),
        }
    }

    /// Decodes until `deadline`, adding a finished batch to the kept frames and
    /// showing frames for backward steps that were waiting
    pub fn work(
        &mut self,
        deadline: Instant,
        current: &mut Option<TimedFrame>,
        pending: &mut Option<TimedFrame>,
        frame_duration: f64,
    ) -> Worked {
        let Some(prefetch) = self.prefetch.as_mut().filter(|prefetch| prefetch.busy())
        else {
            return Worked::default();
        };
        let batch = match prefetch.work(deadline) {
            Ok(Some(batch)) => batch,
            Ok(None) => return Worked::default(),
            Err(error) => {
                self.fail(&error);
                return Worked::default();
            }
        };
        let Some(shown) = current.as_ref() else {
            return Worked::default();
        };
        // A batch that found nothing marks an end of the video
        if batch.frames.is_empty() {
            self.exhausted = Some(batch.direction);
            self.waiting_back = 0;
            return Worked {
                finished: true,
                ..Worked::default()
            };
        }
        let limit = self.history.limit(&shown.frame);
        let tolerance = frame_duration * 0.5;
        let mut worked = Worked {
            finished: true,
            ..Worked::default()
        };
        match batch.direction {
            Direction::Back { before } => {
                let oldest = self.history.oldest().unwrap_or(shown.position);
                if (oldest - before).abs() <= tolerance {
                    self.add_earlier(batch, limit);
                    worked.stepped_back = self.show_waiting(current, pending);
                }
            }
            Direction::Forward { from } => {
                if self.gap.is_some_and(|gap| (gap - from).abs() <= tolerance) {
                    self.add_later(batch, limit, frame_duration);
                    worked.added_ahead = true;
                }
            }
        }
        worked
    }

    /// Keeps the latest frames of a backward batch that fit
    fn add_earlier(&mut self, batch: Batch, limit: usize) {
        let room = limit.saturating_sub(self.history.len());
        let mut frames = batch.frames;
        frames.drain(..frames.len().saturating_sub(room));
        self.history.prepend(frames);
    }

    /// Adds a forward batch ahead, moving the gap to just after it
    fn add_later(&mut self, batch: Batch, limit: usize, frame_duration: f64) {
        let frames = batch.frames;
        let next = frames.last().map(|frame| frame.position + frame_duration);
        self.history.drop_oldest(limit.saturating_sub(frames.len()));
        self.history.append(frames);
        self.gap = next.or(self.gap);
    }

    /// Shows kept frames for backward steps that were waiting; returns how many
    fn show_waiting(
        &mut self,
        current: &mut Option<TimedFrame>,
        pending: &mut Option<TimedFrame>,
    ) -> u32 {
        let shown = self.show_previous(current, pending, self.waiting_back);
        self.waiting_back -= shown;
        shown
    }

    fn fail(&mut self, error: &anyhow::Error) {
        log::info!("Background decoding stopped; backward steps will seek: {error:#}");
        self.prefetch = None;
        self.failed = true;
        self.waiting_back = 0;
    }
}

/// The fewest frames worth a batch while kept frames remain
fn batch_size(limit: usize) -> usize {
    (limit / 3).max(2)
}
