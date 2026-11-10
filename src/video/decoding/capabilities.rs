//! Which decoders this computer can open, for the decoder preference list

use super::backend;
use crate::{settings::VideoDecoder, video::gpu::NativeGpu};

/// The decoder settings that work on this GPU, with Auto first and Software last
///
/// Creating CUDA and VA-API devices takes hundreds of milliseconds, so callers run
/// this away from the UI thread
pub(in crate::video) fn available_decoders(gpu: &NativeGpu) -> Vec<VideoDecoder> {
    let mut decoders = vec![VideoDecoder::Auto];

    for &decoder in backend::candidates(VideoDecoder::Auto) {
        // The temporary device is dropped while this still borrows the live GPU
        if backend::detect(decoder, gpu) {
            decoders.push(decoder);
        }
    }

    decoders.push(VideoDecoder::Software);
    decoders
}
