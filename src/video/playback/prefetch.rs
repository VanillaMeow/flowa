//! Decodes the frames before the oldest kept one while the user steps backward, so
//! backward steps never wait for a seek
//!
//! Frames can only be decoded forward from a keyframe, so earlier frames come in
//! batches: a second decoder on the same device seeks its own reader to before the
//! oldest kept frame and decodes up to it, keeping the last frames that fit. It runs in
//! short slices on the playback thread, between requests from the UI

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
}

/// Frames for before one position
struct Job {
    /// Where the oldest kept frame sits; frames from before it are collected
    before: f64,
    /// How many of the latest collected frames to keep
    count: usize,
    collected: VecDeque<TimedFrame>,
    unsent: Option<Packet>,
    drained: bool,
    last_position: f64,
}

/// Earlier frames, oldest first, for the frames kept from `before` on
pub(super) struct Batch {
    pub before: f64,
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
        })
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn cancel(&mut self) {
        self.job = None;
    }

    /// Starts decoding up to `count` frames before `before`
    ///
    /// When a keyframe falls within those frames, decoding starts there and the batch
    /// stops short of `count`, since earlier frames would mean decoding from the
    /// keyframe before it
    pub fn start(&mut self, before: f64, count: usize) -> Result<()> {
        let frame = self.info.frame_duration;
        let earliest = (before - frame * (count as f64 + 0.5)).max(0.);
        let keyframe = self
            .media
            .keyframe_at_or_before(before - frame * 0.5)
            .filter(|&keyframe| keyframe >= earliest);

        // Seeking a little past an indexed keyframe still lands on it
        let target = keyframe.map_or(earliest, |keyframe| keyframe + frame * 0.25);

        self.job = None;
        self.media.seek(target)?;
        self.decoder.flush();
        self.job = Some(Job {
            before,
            count,
            collected: VecDeque::with_capacity(count + 1),
            unsent: None,
            drained: false,
            last_position: earliest - frame,
        });

        Ok(())
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

    /// Takes the frames the decoder has ready, finishing the batch once one reaches
    /// the kept frames
    fn receive(&mut self) -> Result<Option<Batch>> {
        let Some(job) = &mut self.job else {
            return Ok(None);
        };

        loop {
            let frame = match self.decoder.receive()? {
                Receive::Item(frame) => frame,
                Receive::Again => return Ok(None),
                Receive::End => return Ok(self.finish()),
            };
            let position = ffmpeg::timestamp(
                frame.info().timestamp,
                self.info.video_time_base,
                self.info.origin,
            )
            .unwrap_or(job.last_position + self.info.frame_duration);
            job.last_position = position;

            if position + self.info.frame_duration * 0.5 >= job.before {
                return Ok(self.finish());
            }

            job.collected.push_back(TimedFrame { frame, position });
            if job.collected.len() > job.count {
                job.collected.pop_front();
            }
        }
    }

    /// Offers the decoder the next video packet, or drains it at the end
    fn send(&mut self) -> Result<()> {
        let Some(job) = &mut self.job else {
            return Ok(());
        };

        if job.drained {
            return Ok(());
        }

        let packet = match job.unsent.take() {
            Some(packet) => Some(packet),
            None => next_video_packet(&mut self.media)?,
        };

        let Some(packet) = packet else {
            self.decoder.send(None)?;
            job.drained = true;
            return Ok(());
        };

        if let Sent::Again = self.decoder.send(Some(&packet))? {
            job.unsent = Some(packet);
        }

        Ok(())
    }

    fn finish(&mut self) -> Option<Batch> {
        let job = self.job.take()?;

        Some(Batch {
            before: job.before,
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
