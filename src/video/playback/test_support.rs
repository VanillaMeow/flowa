//! Helpers shared by the playback tests

use anyhow::Result;
use eframe::egui_wgpu::{RenderState, RendererOptions, WgpuConfiguration};

use crate::{settings::VideoDecoder, video::gpu};

/// A rendering device set up like the app's, without a window
pub(super) fn test_graphics() -> Result<RenderState> {
    let setup = gpu::setup()?;
    let instance = pollster::block_on(setup.new_instance());
    let config = WgpuConfiguration {
        wgpu_setup: setup,
        ..Default::default()
    };
    Ok(pollster::block_on(RenderState::create(
        &config,
        &instance,
        None,
        RendererOptions::default(),
    ))?)
}

/// The decoder a name such as `nvdec` stands for
pub(super) fn named_decoder(name: &str) -> Option<VideoDecoder> {
    match name.to_lowercase().as_str() {
        "auto" => Some(VideoDecoder::Auto),
        "nvdec" => Some(VideoDecoder::Nvdec),
        "vulkan" => Some(VideoDecoder::Vulkan),
        "vaapi" => Some(VideoDecoder::Vaapi),
        "software" => Some(VideoDecoder::Software),
        _ => None,
    }
}

/// The decoder `FLOWA_TEST_VIDEO_DECODER` names, if any
pub(super) fn chosen_decoder() -> Option<VideoDecoder> {
    named_decoder(&std::env::var("FLOWA_TEST_VIDEO_DECODER").ok()?)
}
