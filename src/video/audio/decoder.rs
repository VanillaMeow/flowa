//! Decodes the audio stream into blocks of output-format samples

use anyhow::Result;

use super::resampler::Resampler;
use crate::video::{
    ffi::ffmpeg as ffi,
    ffmpeg::{
        CodecContext, FfmpegDefaults, Packet, Receive, Sent, StreamInfo, timestamp,
    },
};

pub(in crate::video) struct AudioDecoder {
    codec: CodecContext,
    resampler: Resampler,
}

impl AudioDecoder {
    pub fn open(stream: &ffi::AVStream) -> Result<Self> {
        let codec = CodecContext::open(stream, FfmpegDefaults)?;
        let resampler = Resampler::new(codec.data())?;

        Ok(Self { codec, resampler })
    }

    pub fn flush(&mut self) -> Result<()> {
        self.codec.flush();
        self.resampler.reset()
    }

    pub fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        self.codec.send(packet)
    }

    pub fn receive(&mut self) -> Result<Receive<AudioBlock>> {
        let frame = match self.codec.receive()? {
            Receive::Again => return Ok(Receive::Again),
            Receive::End => return Ok(Receive::End),
            Receive::Item(frame) => frame,
        };
        let resampled = self.resampler.convert(&frame)?;

        Ok(Receive::Item(AudioBlock {
            samples: resampled.samples,
            timestamp: frame.data().best_effort_timestamp,
            resample_delay: resampled.delay,
        }))
    }
}

pub(in crate::video) struct AudioBlock {
    samples: Vec<f32>,
    timestamp: i64,
    resample_delay: f64,
}

impl AudioBlock {
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn position(&self, info: &StreamInfo, previous: f64) -> f64 {
        timestamp(self.timestamp, info.audio_time_base, info.origin).unwrap_or(previous)
            - self.resample_delay
    }
}
