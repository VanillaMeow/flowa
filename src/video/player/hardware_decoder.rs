//! Hardware decoding on the rendering GPU, with frames that never leave it

use std::sync::Arc;

use anyhow::Result;

use super::{decoded_frame::Frame, decoder::Decoder};
use crate::video::{
    gpu::{DecoderDevice, NativeGpu},
    media::{Codec, Packet, Receive, Sent, Stream},
};

/// Decodes with NVDEC, Vulkan Video, or VA-API on the GPU that renders the frames
///
/// Keeps the rendering device alive until the codec and the FFmpeg device it borrows
/// are released
pub(super) struct HardwareDecoder {
    codec: Codec,
    gpu: Arc<NativeGpu>,
}

impl HardwareDecoder {
    pub fn new(
        gpu: &Arc<NativeGpu>,
        stream: &Stream,
        device: DecoderDevice,
    ) -> Result<Self> {
        let device = gpu.decoder_device(device)?;
        let codec = Codec::hardware(stream, device)?;

        Ok(Self {
            codec,
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

    fn receive(&mut self) -> Result<Receive<Frame>> {
        match self.codec.receive()? {
            Receive::Again => Ok(Receive::Again),
            Receive::End => Ok(Receive::End),
            Receive::Item(frame) => {
                Ok(Receive::Item(Frame::hardware(frame, Arc::clone(&self.gpu))))
            }
        }
    }
}
