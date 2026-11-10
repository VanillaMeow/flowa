//! What every video decoder provides, and the backends that implement it

use std::sync::Arc;

use anyhow::Result;
use strum::{IntoStaticStr, VariantArray};

use super::{
    decoded_frame::Frame, hardware_decoder::HardwareDecoder,
    software_decoder::SoftwareDecoder,
};
use crate::{
    settings::VideoDecoder,
    video::{
        gpu::{DecoderDevice, NativeGpu},
        media::{Packet, Receive, Sent, Stream},
    },
};

/// A video decoder: packets go in and frames come out, without blocking playback
///
/// `HardwareDecoder` decodes on the rendering GPU and `SoftwareDecoder` on a CPU
/// thread. `DecoderSelection` opens one of them through `Backend::open` and replaces
/// it with the next backend when it fails
pub(super) trait Decoder {
    /// Discards queued packets and frames so decoding can restart from a keyframe
    fn flush(&mut self);

    /// Offers the next packet, or `None` to drain the decoder at the end of the stream
    ///
    /// `Sent::Again` means the decoder is full and the same packet must be offered again
    fn send(&mut self, packet: Option<&Packet>) -> Result<Sent>;

    /// Takes the next frame, or reports that more packets are needed or the stream ended
    fn receive(&mut self) -> Result<Receive<Frame>>;
}

/// The decoders flowa can open, in the order Auto tries them
#[derive(Clone, Copy, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "lowercase")]
pub(super) enum Backend {
    Nvdec,
    Vulkan,
    Vaapi,
    Software,
}

impl Backend {
    pub fn available(self) -> bool {
        !matches!(self, Self::Vaapi) || cfg!(target_os = "linux")
    }

    pub fn candidates(preference: VideoDecoder) -> &'static [Self] {
        match preference {
            VideoDecoder::Auto => Self::VARIANTS,
            // Vulkan Video runs on any GPU with a video queue, so it backs up the others
            VideoDecoder::Nvdec => &[Self::Nvdec, Self::Vulkan, Self::Software],
            VideoDecoder::Vulkan => &[Self::Vulkan, Self::Software],
            VideoDecoder::Vaapi => &[Self::Vaapi, Self::Vulkan, Self::Software],
            VideoDecoder::Software => &[Self::Software],
        }
    }

    /// The GPU device the decoder needs, or `None` for FFmpeg's own software decoders
    pub fn device(self) -> Option<DecoderDevice> {
        match self {
            Self::Nvdec => Some(DecoderDevice::Nvdec),
            Self::Vulkan => Some(DecoderDevice::Vulkan),
            Self::Vaapi => Some(DecoderDevice::Vaapi),
            Self::Software => None,
        }
    }

    pub fn preference(self) -> VideoDecoder {
        match self {
            Self::Nvdec => VideoDecoder::Nvdec,
            Self::Vulkan => VideoDecoder::Vulkan,
            Self::Vaapi => VideoDecoder::Vaapi,
            Self::Software => VideoDecoder::Software,
        }
    }

    /// Opens this backend's decoder for a stream
    pub fn open(
        self,
        gpu: &Arc<NativeGpu>,
        stream: &Arc<Stream>,
    ) -> Result<Box<dyn Decoder>> {
        let Some(device) = self.device() else {
            return Ok(Box::new(SoftwareDecoder::new(Arc::clone(stream))?));
        };

        Ok(Box::new(HardwareDecoder::new(gpu, stream, device)?))
    }
}

#[cfg(test)]
mod tests {
    use super::Backend;
    use crate::settings::VideoDecoder;
    use strum::VariantArray;

    #[test]
    fn auto_tries_hardware_backends_before_software() {
        let names: Vec<&'static str> =
            Backend::VARIANTS.iter().copied().map(Into::into).collect();

        assert_eq!(names, ["nvdec", "vulkan", "vaapi", "software"]);
    }

    #[test]
    fn an_explicit_decoder_falls_back_through_vulkan_to_software() {
        for (preference, expected) in [
            (VideoDecoder::Nvdec, vec!["nvdec", "vulkan", "software"]),
            (VideoDecoder::Vulkan, vec!["vulkan", "software"]),
            (VideoDecoder::Vaapi, vec!["vaapi", "vulkan", "software"]),
            (VideoDecoder::Software, vec!["software"]),
        ] {
            let names: Vec<&'static str> = Backend::candidates(preference)
                .iter()
                .copied()
                .map(Into::into)
                .collect();
            assert_eq!(names, expected);
        }
    }
}
