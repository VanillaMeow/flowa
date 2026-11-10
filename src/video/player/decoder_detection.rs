//! Finds out which hardware decoders this computer can open

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use strum::VariantArray;

use super::decoder::Backend;
use crate::{settings::VideoDecoder, video::gpu::NativeGpu};

/// Detects hardware decoders on a background thread the first time the list is needed
///
/// Creating CUDA and VA-API devices takes hundreds of milliseconds, so startup skips it
pub(super) struct DecoderDetection {
    gpu: Arc<NativeGpu>,
    ctx: egui::Context,
    started: AtomicBool,
    decoders: Arc<OnceLock<Vec<VideoDecoder>>>,
}

impl DecoderDetection {
    pub fn new(gpu: Arc<NativeGpu>, ctx: egui::Context) -> Self {
        Self {
            gpu,
            ctx,
            started: AtomicBool::new(false),
            decoders: Arc::new(OnceLock::new()),
        }
    }

    /// The available decoders, or `None` while detection is still running
    pub fn decoders(&self) -> Option<&[VideoDecoder]> {
        if !self.started.swap(true, Ordering::Relaxed) {
            self.start();
        }

        self.decoders.get().map(Vec::as_slice)
    }

    fn start(&self) {
        let gpu = Arc::clone(&self.gpu);
        let decoders = Arc::clone(&self.decoders);
        let ctx = self.ctx.clone();

        let spawned = thread::Builder::new()
            .name("flowa-decoder-probe".into())
            .spawn(move || {
                let _ = decoders.set(available_decoders(&gpu));
                ctx.request_repaint();
            });

        if let Err(error) = spawned {
            log::warn!("Could not detect hardware decoders: {error}");
            let _ = self
                .decoders
                .set(vec![VideoDecoder::Auto, VideoDecoder::Software]);
        }
    }
}

fn available_decoders(gpu: &NativeGpu) -> Vec<VideoDecoder> {
    let mut decoders = vec![VideoDecoder::Auto];

    for &backend in Backend::VARIANTS {
        let Some(device) = backend.device().filter(|_| backend.available()) else {
            continue;
        };

        // The temporary device is dropped while this still borrows the live GPU
        if gpu.decoder_device(device).is_ok() {
            decoders.push(backend.preference());
        }
    }

    decoders.push(VideoDecoder::Software);
    decoders
}
