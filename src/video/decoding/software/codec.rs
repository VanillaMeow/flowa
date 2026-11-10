//! FFmpeg's CPU video decoders, configured for playback or for a single poster frame

use anyhow::{Result, ensure};

use crate::video::{
    decoding::frame::CpuFrame,
    ffi::ffmpeg as ffi,
    ffmpeg::{CodecContext, CodecSetup, Packet, Receive, Sent},
};

/// How many threads a software decoder uses
#[derive(Clone, Copy)]
pub(in crate::video) enum Threads {
    /// Lets FFmpeg pick frame and slice threads for sustained playback
    Automatic,
    /// Avoids thread startup and frame delay when decoding a single poster frame
    Single,
}

impl CodecSetup for Threads {
    fn configure(&mut self, context: &mut ffi::AVCodecContext) {
        // A zero thread count asks FFmpeg to size the pool from the CPU
        context.thread_count = match self {
            Self::Automatic => 0,
            Self::Single => 1,
        };
        context.thread_type = (ffi::FF_THREAD_FRAME | ffi::FF_THREAD_SLICE) as i32;
    }
}

/// A CPU video decoder that hands out frames that are safe to move between threads
pub(in crate::video) struct SoftwareCodec(CodecContext);

impl SoftwareCodec {
    pub fn open(stream: &ffi::AVStream, threads: Threads) -> Result<Self> {
        Ok(Self(CodecContext::open(stream, threads)?))
    }

    pub fn flush(&mut self) {
        self.0.flush();
    }

    /// Sends one packet, or `None` to drain, and passes on every frame it releases
    ///
    /// Returns false once `output` declines further frames
    pub fn decode(
        &mut self,
        packet: Option<&Packet>,
        mut output: impl FnMut(Receive<CpuFrame>) -> bool,
    ) -> Result<bool> {
        ensure!(
            !matches!(self.0.send(packet)?, Sent::Again),
            "Software decoder did not consume its previous output"
        );

        loop {
            let frame = match self.0.receive()? {
                Receive::Again => return Ok(true),
                Receive::End => return Ok(output(Receive::End)),
                Receive::Item(frame) => Receive::Item(CpuFrame::new(frame)?),
            };

            if !output(frame) {
                return Ok(false);
            }
        }
    }
}
