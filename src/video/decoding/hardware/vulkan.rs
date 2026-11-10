//! Vulkan Video on the shared device's video decode queue

use anyhow::{Context, Result, ensure};

use super::HardwareApi;
use crate::video::{
    decoding::HardwareStorage, ffi::ffmpeg as ffi, ffmpeg::HwDevice, gpu::NativeGpu,
};

pub(in crate::video::decoding) struct VulkanVideo;

impl HardwareApi for VulkanVideo {
    fn storage(&self) -> HardwareStorage {
        HardwareStorage::Vulkan
    }

    fn create_device(&self, gpu: &NativeGpu) -> Result<HwDevice> {
        let device = gpu.device();
        ensure!(
            device.has_video_queue(),
            "The rendering GPU has no Vulkan video decode queue"
        );
        let shared = device
            .ffmpeg_device()
            .context("FFmpeg cannot use the rendering device")?;

        // SAFETY: Referencing the shared device leaves it unchanged
        unsafe { HwDevice::from_raw(ffi::av_buffer_ref(shared)) }
    }
}
