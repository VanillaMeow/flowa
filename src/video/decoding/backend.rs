//! How each decoder setting opens, and the order Auto and fallback try them in
//!
//! `VideoDecoder` is the one list of decoders. `hardware` is the only place that maps a
//! setting to its implementation

use std::sync::Arc;

use anyhow::{Result, ensure};

use super::{
    Decoder,
    hardware::{HardwareApi, HardwareDecoder, Nvdec, Vaapi, VulkanVideo},
    software::SoftwareDecoder,
};
use crate::{
    settings::VideoDecoder,
    video::{ffmpeg::Stream, gpu::NativeGpu},
};

/// The decoders to try for a preference, in order
pub(super) fn candidates(preference: VideoDecoder) -> &'static [VideoDecoder] {
    use VideoDecoder::{Auto, Nvdec, Software, Vaapi, Vulkan};

    match preference {
        Auto => &[Nvdec, Vulkan, Vaapi, Software],
        // Vulkan Video runs on any GPU with a video queue, so it backs up the others
        Nvdec => &[Nvdec, Vulkan, Software],
        Vulkan => &[Vulkan, Software],
        Vaapi => &[Vaapi, Vulkan, Software],
        Software => &[Software],
    }
}

/// The hardware API behind a decoder, or `None` for software decoding and Auto
fn hardware(decoder: VideoDecoder) -> Option<&'static dyn HardwareApi> {
    match decoder {
        VideoDecoder::Nvdec => Some(&Nvdec),
        VideoDecoder::Vulkan => Some(&VulkanVideo),
        VideoDecoder::Vaapi => Some(&Vaapi),
        VideoDecoder::Auto | VideoDecoder::Software => None,
    }
}

pub(super) fn is_hardware(decoder: VideoDecoder) -> bool {
    hardware(decoder).is_some()
}

/// Whether flowa supports the decoder on this platform
pub(super) fn available(decoder: VideoDecoder) -> bool {
    hardware(decoder).is_none_or(|api| api.available())
}

/// Whether a hardware decoder can open on this GPU, by creating and dropping its device
pub(super) fn detect(decoder: VideoDecoder, gpu: &NativeGpu) -> bool {
    hardware(decoder)
        .is_some_and(|api| api.available() && api.create_device(gpu).is_ok())
}

/// Opens a decoder for a stream
pub(super) fn open(
    decoder: VideoDecoder,
    gpu: &Arc<NativeGpu>,
    stream: &Arc<Stream>,
) -> Result<Box<dyn Decoder>> {
    ensure!(
        decoder != VideoDecoder::Auto,
        "Auto chooses a decoder rather than being one"
    );

    let Some(api) = hardware(decoder) else {
        return Ok(Box::new(SoftwareDecoder::new(Arc::clone(stream))?));
    };

    Ok(Box::new(HardwareDecoder::open(api, gpu, stream)?))
}

#[cfg(test)]
mod tests {
    use super::candidates;
    use crate::settings::VideoDecoder;

    fn names(decoders: &[VideoDecoder]) -> Vec<&str> {
        decoders.iter().map(AsRef::as_ref).collect()
    }

    #[test]
    fn auto_tries_hardware_decoders_before_software() {
        assert_eq!(
            names(candidates(VideoDecoder::Auto)),
            ["NVDEC", "Vulkan", "VA-API", "Software"]
        );
    }

    #[test]
    fn an_explicit_decoder_falls_back_through_vulkan_to_software() {
        for (preference, expected) in [
            (VideoDecoder::Nvdec, vec!["NVDEC", "Vulkan", "Software"]),
            (VideoDecoder::Vulkan, vec!["Vulkan", "Software"]),
            (VideoDecoder::Vaapi, vec!["VA-API", "Vulkan", "Software"]),
            (VideoDecoder::Software, vec!["Software"]),
        ] {
            assert_eq!(names(candidates(preference)), expected);
        }
    }
}
