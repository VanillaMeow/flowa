//! Audio decoding, resampling to the output format, and SDL3 output

mod decoder;
mod output;
mod resampler;

pub(super) use decoder::{AudioBlock, AudioDecoder};
pub(super) use output::{AudioOutput, AudioRuntime};

/// The output sample rate every source is resampled to
pub(super) const RATE: usize = 48_000;
/// Interleaved stereo output
pub(super) const CHANNELS: usize = 2;
pub(super) const BYTES_PER_SECOND: usize = RATE * CHANNELS * size_of::<f32>();
