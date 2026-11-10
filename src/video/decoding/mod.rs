//! Video decoders: the contract every decoder fulfils, and the software and hardware
//! implementations behind it
//!
//! `SoftwareDecoder` decodes on a CPU thread. `HardwareDecoder` decodes on the
//! rendering GPU through one of the APIs in `hardware/`. `DecoderSelection` opens the
//! first decoder that works and replaces it with the next one when it fails

use anyhow::Result;

use super::ffmpeg::{Packet, Receive, Sent};

mod backend;
mod capabilities;
mod frame;
mod hardware;
mod selection;
mod software;

pub(super) use capabilities::available_decoders;
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
}
