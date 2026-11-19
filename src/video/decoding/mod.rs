//! Video decoders: the contract every decoder fulfils, and the software and hardware
//! implementations behind it
//!
//! `SoftwareDecoder` decodes on a CPU thread. `HardwareDecoder` decodes on the
//! rendering GPU through one of the APIs in `hardware/`. `DecoderSelection` opens the
//! first decoder that works and replaces it with the next one when it fails

use anyhow::Result;

use super::ffmpeg::{HwDevice, Packet, Receive, Sent};

mod backend;
mod capabilities;
mod frame;
mod hardware;
mod selection;
mod software;

pub(super) use capabilities::available_decoders;

/// Frames kept around the shown one while playing, so the first backward steps after
/// pausing are instant, bounded by count and by memory
const PLAYING_KEPT: KeptFrames = KeptFrames {
    budget: 128 * 1024 * 1024,
    frames: 4..=8,
};
/// Frames kept while stepping frame by frame, so background batches have room
const STEPPING_KEPT: KeptFrames = KeptFrames {
    budget: 256 * 1024 * 1024,
    frames: 8..=24,
};
/// Further frames held at once: the shown frame, the next one, and frames the GPU is
/// still reading
const HELD_HARDWARE_FRAMES: usize = 4;

struct KeptFrames {
    budget: usize,
    frames: std::ops::RangeInclusive<usize>,
}

/// How many decoded frames of this size playback may keep around the shown one;
/// decoders whose surface pool cannot grow add surfaces for the stepping count
pub(super) fn kept_frames(width: i32, height: i32, stepping: bool) -> usize {
    let kept = if stepping {
        STEPPING_KEPT
    } else {
        PLAYING_KEPT
    };
    let pixels = (width.max(1) as usize) * (height.max(1) as usize);
    // Up to two bytes per pixel covers 4:2:0 video at ten or more bits
    (kept.budget / (pixels * 2)).clamp(*kept.frames.start(), *kept.frames.end())
}

pub(super) use frame::{DecodedFrame, HardwareFrame, HardwareStorage};
pub(super) use selection::{DecoderSelection, PacketSource, SourcePacket};
pub(super) use software::{SoftwareCodec, Threads};

/// A video decoder: packets go in and frames come out, without blocking playback
pub(super) trait Decoder {
    /// Discards queued packets and frames so decoding can restart from a keyframe
    fn flush(&mut self);

    /// Offers the next packet, or `None` to drain the decoder at the end of the stream
    ///
    /// `Sent::Again` means the decoder is full and the same packet must be offered again
    fn send(&mut self, packet: Option<&Packet>) -> Result<Sent>;

    /// Takes the next frame, or reports that more packets are needed or the stream ended
    fn receive(&mut self) -> Result<Receive<DecodedFrame>>;

    /// The hardware device this decoder runs on, which a second decoder can share
    fn device(&self) -> Option<HwDevice> {
        None
    }
}
