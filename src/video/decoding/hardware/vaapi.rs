//! VA-API on the rendering GPU's DRM render node

use std::ptr;

use anyhow::{Context, Result};

use super::HardwareApi;
use crate::video::{
    decoding::HardwareStorage,
    ffi::ffmpeg as ffi,
    ffmpeg::{HwDevice, check},
    gpu::NativeGpu,
};

pub(in crate::video::decoding) struct Vaapi;

impl HardwareApi for Vaapi {
    fn storage(&self) -> HardwareStorage {
        HardwareStorage::Vaapi
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
    }

    fn create_device(&self, gpu: &NativeGpu) -> Result<HwDevice> {
        let node = gpu
            .device()
            .render_node()
            .context("The rendering GPU has no DRM render node")?;
        let mut device = ptr::null_mut();

        // SAFETY: FFmpeg only reads the path and leaves the output unset on failure
        unsafe {
            check(
                ffi::av_hwdevice_ctx_create(
                    &mut device,
                    ffi::AV_HWDEVICE_TYPE_VAAPI,
                    node.as_ptr(),
                    ptr::null_mut(),
                    0,
                ),
                "Cannot open VA-API on the rendering GPU",
            )?;

            HwDevice::from_raw(device)
        }
    }
}
