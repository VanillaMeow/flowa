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

/// Memory decoded frames held for stepping back may take
const RETAINED_BUDGET: usize = 256 * 1024 * 1024;
const MIN_RETAINED_FRAMES: usize = 12;
const MAX_RETAINED_FRAMES: usize = 32;
/// Further frames held at once: the shown frame, the next one, and frames the GPU is
/// still reading
const HELD_HARDWARE_FRAMES: usize = 4;

/// How many decoded frames of this size playback may hold beyond what decoding needs,
/// for stepping back without decoding again; decoders whose surface pool cannot grow
/// add this many surfaces
pub(super) fn retained_frames(width: i32, height: i32) -> usize {
    let pixels = (width.max(1) as usize) * (height.max(1) as usize);

    // Up to two bytes per pixel covers 4:2:0 video at ten or more bits
    (RETAINED_BUDGET / (pixels * 2)).clamp(MIN_RETAINED_FRAMES, MAX_RETAINED_FRAMES)
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
