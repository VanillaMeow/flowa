//! A second decoder that fills in frames around the shown one while the user steps
//!
//! Frames can only be decoded forward from a keyframe, so they come in batches. Going
//! back, the decoder seeks its own reader to before the oldest kept frame and decodes
//! up to it, keeping the last frames that fit. Going forward past frames dropped to
//! make room, it decodes on from where they stopped, and a batch that starts where the
//! previous forward batch ended carries on without seeking. Batches run in short
//! slices on the playback thread, between requests from the UI

use std::{collections::VecDeque, path::Path, sync::Arc, time::Instant};

use anyhow::Result;

use super::frame_history::TimedFrame;
use crate::video::{
    decoding::Decoder,
    demux::{InterruptFlags, Media},
    ffmpeg::{self, Packet, PacketKind, Receive, Sent, StreamInfo},
};

pub(super) struct Prefetch {
    media: Media,
    decoder: Box<dyn Decoder>,
    info: StreamInfo,
    job: Option<Job>,
    /// Where the next decoded frame sits after a finished forward batch, so the next
    /// one can continue without seeking
    continues_at: Option<f64>,
    /// A packet the decoder had no room for, which belongs to its position rather than
    /// to any one batch
    unsent: Option<Packet>,
    /// Set once the reader ran out and the decoder was drained
    drained: bool,
}

/// Which side of the kept frames a batch fills
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Direction {
    /// Frames before the oldest kept frame, which sits here
    Back { before: f64 },
    /// Frames from here on, after the last kept frame ahead
    Forward { from: f64 },
}

struct Job {
    direction: Direction,
    /// How many frames to keep: the latest going back, the first going forward
    count: usize,
    collected: VecDeque<TimedFrame>,
    last_position: f64,
}

/// Decoded frames in display order, for the side `direction` names
pub(super) struct Batch {
    pub direction: Direction,
    pub frames: Vec<TimedFrame>,
}

impl Prefetch {
    pub fn open(
        path: &Path,
        info: StreamInfo,
        decoder: Box<dyn Decoder>,
    ) -> Result<Self> {
        let mut media = Media::open(path, Arc::new(InterruptFlags::default()))?;
        media.read_only_video();
        Ok(Self {
            media,
            decoder,
            info,
            job: None,
            continues_at: None,
            unsent: None,
            drained: false,
        })
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn cancel(&mut self) {
        self.job = None;
        self.continues_at = None;
    }

    /// Starts decoding `count` frames on the side `direction` names
    pub fn start(&mut self, direction: Direction, count: usize) -> Result<()> {
        let frame = self.info.frame_duration;
        // A forward batch right after the last one keeps the decoder's position
        let continuing = matches!(
            (direction, self.continues_at),
            (Direction::Forward { from }, Some(next)) if (from - next).abs() < frame * 0.5
        );
        let first = match direction {
            Direction::Back { before } => self.back_start(before, count),
            Direction::Forward { from } => from,
        };
        self.job = None;
        self.continues_at = None;
        if !continuing {
            // Seeking a little past an indexed keyframe still lands on it
            self.media.seek(first + frame * 0.25)?;
            self.decoder.flush();
            self.unsent = None;
            self.drained = false;
        }
        self.job = Some(Job {
            direction,
            count,
            collected: VecDeque::with_capacity(count + 1),
            last_position: first - frame,
        });
        Ok(())
    }

    /// Where a backward batch starts: at an indexed keyframe when one falls within
    /// it, since earlier frames would mean decoding from the keyframe before that one
    fn back_start(&self, before: f64, count: usize) -> f64 {
        let frame = self.info.frame_duration;
        let earliest = (before - frame * (count as f64 + 0.5)).max(0.);
        self.media
            .keyframe_at_or_before(before - frame * 0.5)
            .filter(|&keyframe| keyframe >= earliest)
            .unwrap_or(earliest)
    }

    /// Decodes until the batch is complete or `deadline` passes
    pub fn work(&mut self, deadline: Instant) -> Result<Option<Batch>> {
        while self.job.is_some() && Instant::now() < deadline {
            if let Some(batch) = self.receive()? {
                return Ok(Some(batch));
            }
            self.send()?;
        }
        Ok(None)
    }

    /// Takes the frames the decoder has ready, finishing the batch once it has what
    /// it needs
    fn receive(&mut self) -> Result<Option<Batch>> {
        let frame_duration = self.info.frame_duration;
        loop {
            let Some(job) = &mut self.job else {
                return Ok(None);
            };
            let frame = match self.decoder.receive()? {
                Receive::Item(frame) => frame,
                Receive::Again => return Ok(None),
                Receive::End => return Ok(self.finish(None)),
            };
            let timestamp = frame.info().timestamp;
            let position = ffmpeg::timestamp(
                timestamp,
                self.info.video_time_base,
                self.info.origin,
            )
            .unwrap_or(job.last_position + frame_duration);
            job.last_position = position;
            match job.direction {
                // Going back, the batch ends at the kept frames and keeps the latest
                Direction::Back { before } => {
                    if position + frame_duration * 0.5 >= before {
                        return Ok(self.finish(None));
                    }
                    job.collected.push_back(TimedFrame { frame, position });
                    if job.collected.len() > job.count {
                        job.collected.pop_front();
                    }
                }
                // Going forward, frames before the gap only lead up to it
                Direction::Forward { from } => {
                    if position + frame_duration * 0.5 < from {
                        continue;
                    }
                    job.collected.push_back(TimedFrame { frame, position });
                    if job.collected.len() == job.count {
                        return Ok(self.finish(Some(position + frame_duration)));
                    }
                }
            }
        }
    }

    /// Offers the decoder the next video packet, or drains it at the end
    fn send(&mut self) -> Result<()> {
        if self.job.is_none() || self.drained {
            return Ok(());
        }
        let packet = match self.unsent.take() {
            Some(packet) => Some(packet),
            None => next_video_packet(&mut self.media)?,
        };
        let Some(packet) = packet else {
            self.decoder.send(None)?;
            self.drained = true;
            return Ok(());
        };
        if let Sent::Again = self.decoder.send(Some(&packet))? {
            self.unsent = Some(packet);
        }
        Ok(())
    }

    /// Hands over the collected frames; `continues_at` is where the decoder stands for
    /// a following forward batch
    fn finish(&mut self, continues_at: Option<f64>) -> Option<Batch> {
        let job = self.job.take()?;
        self.continues_at = continues_at;
        Some(Batch {
            direction: job.direction,
            frames: job.collected.into(),
        })
    }
}

fn next_video_packet(media: &mut Media) -> Result<Option<Packet>> {
    while let Some(packet) = media.read()? {
        if matches!(packet.kind, PacketKind::Video) {
            return Ok(Some(packet));
        }
    }
    Ok(None)
}
