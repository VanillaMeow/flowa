//! NVDEC through a CUDA device derived from the shared Vulkan device

use std::ptr;

use anyhow::{Context, Result};

use super::HardwareApi;
use crate::video::{
    decoding::HardwareStorage,
    ffi::ffmpeg as ffi,
    ffmpeg::{HwDevice, check},
    gpu::NativeGpu,
};

pub(in crate::video::decoding) struct Nvdec;

impl HardwareApi for Nvdec {
    fn storage(&self) -> HardwareStorage {
        HardwareStorage::Cuda
    }

    fn create_device(&self, gpu: &NativeGpu) -> Result<HwDevice> {
        let shared = gpu
            .device()
            .ffmpeg_device()
            .context("FFmpeg cannot use the rendering device")?;
        let mut device = ptr::null_mut();

        // SAFETY: FFmpeg only reads the shared device and leaves the output unset on failure
        unsafe {
            check(
                ffi::av_hwdevice_ctx_create_derived(
                    &mut device,
                    ffi::AV_HWDEVICE_TYPE_CUDA,
                    shared,
                    0,
                ),
                "Cannot create a CUDA device on the rendering GPU",
            )?;

            HwDevice::from_raw(device)
        }
    }
}
