//! Hardware decoding on the rendering GPU, with frames that never leave it
//!
//! `HardwareDecoder` is shared by every API. Each API only says how to create its
//! FFmpeg device on the rendering GPU and where its frames end up

use std::sync::Arc;

use anyhow::{Result, ensure};

use super::{DecodedFrame, Decoder, HardwareFrame, HardwareStorage};
use crate::video::{
    ffi::ffmpeg as ffi,
    ffmpeg::{CodecContext, HwDevice, Packet, Receive, Sent, Stream},
    gpu::NativeGpu,
};

mod format;
mod nvdec;
mod vaapi;
mod vulkan;

use format::HardwareFormat;
pub(super) use nvdec::Nvdec;
pub(super) use vaapi::Vaapi;
pub(super) use vulkan::VulkanVideo;

/// What a hardware decoding API provides on top of FFmpeg's generic hardware decoding
pub(super) trait HardwareApi: Sync {
    /// Where the API leaves decoded frames
    fn storage(&self) -> HardwareStorage;

    /// Whether flowa supports the API on this platform
    fn available(&self) -> bool {
        true
    }

    /// Creates FFmpeg's device for the API on the rendering GPU
    fn create_device(&self, gpu: &NativeGpu) -> Result<HwDevice>;
}

/// Decodes with a hardware API on the GPU that renders the frames
///
/// Keeps the rendering device alive until the codec and the FFmpeg device it borrows
/// are released
pub(super) struct HardwareDecoder {
    codec: CodecContext,
    format: ffi::AVPixelFormat,
    storage: HardwareStorage,
    gpu: Arc<NativeGpu>,
}

impl HardwareDecoder {
    pub fn open(
        api: &dyn HardwareApi,
        gpu: &Arc<NativeGpu>,
        stream: &Stream,
    ) -> Result<Self> {
        let device = api.create_device(gpu)?;
        let setup = HardwareFormat::negotiate(stream.data(), device)?;
        let format = setup.format();
        let codec = CodecContext::open(stream.data(), setup)?;

        Ok(Self {
            codec,
            format,
            storage: api.storage(),
            gpu: Arc::clone(gpu),
        })
    }
}

impl Decoder for HardwareDecoder {
    fn flush(&mut self) {
        self.codec.flush();
    }

    fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        self.codec.send(packet)
    }

    fn receive(&mut self) -> Result<Receive<DecodedFrame>> {
        let frame = match self.codec.receive()? {
            Receive::Again => return Ok(Receive::Again),
            Receive::End => return Ok(Receive::End),
            Receive::Item(frame) => frame,
        };

        ensure!(
            !frame.data().hw_frames_ctx.is_null() && frame.data().format == self.format,
            "The hardware decoder returned an unexpected frame format"
        );

        Ok(Receive::Item(DecodedFrame::Hardware(HardwareFrame::new(
            frame,
            self.storage,
            Arc::clone(&self.gpu),
        ))))
    }
}
