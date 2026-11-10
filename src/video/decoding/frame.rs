//! Decoded frames, which live in CPU memory or on the rendering GPU

use std::sync::Arc;

use anyhow::{Result, ensure};

use crate::video::{
    ffi::ffmpeg as ffi,
    ffmpeg::{AvFrame, FrameInfo},
    gpu::NativeGpu,
};

/// A frame from either kind of decoder, ready for presentation
pub(in crate::video) enum DecodedFrame {
    Cpu(CpuFrame),
    Hardware(HardwareFrame),
}

impl DecodedFrame {
    pub fn av_frame(&self) -> &AvFrame {
        match self {
            Self::Cpu(frame) => &frame.0,
            Self::Hardware(frame) => &frame.frame,
        }
    }

    pub fn info(&self) -> FrameInfo {
        self.av_frame().info()
    }

    pub fn display_size(&self, stream_rotation: f64) -> [u32; 2] {
        let info = self.info();
        let aspect = if info.aspect_num > 0 && info.aspect_den > 0 {
            f64::from(info.aspect_num) / f64::from(info.aspect_den)
        } else {
            1.0
        };
        let mut size = [
            (f64::from(info.width.max(0)) * aspect).round() as u32,
            info.height.max(0) as u32,
        ];
        let rotation = if info.rotation.is_finite() {
            info.rotation
        } else {
            stream_rotation
        };
        let quarter_turns = (rotation.abs() / 90.0).round() as u32;

        if rotation.is_finite() && !quarter_turns.is_multiple_of(2) {
            size.swap(0, 1);
        }

        size
    }
}

/// A frame in CPU memory, checked to hold no hardware storage so it can change threads
pub(in crate::video) struct CpuFrame(AvFrame);

// SAFETY: Construction rejects hardware storage and transfers one owned software frame
unsafe impl Send for CpuFrame {}

impl CpuFrame {
    pub fn new(frame: AvFrame) -> Result<Self> {
        let data = frame.data();

        // SAFETY: FFmpeg returns a static descriptor for a supported frame format
        let descriptor = unsafe { ffi::av_pix_fmt_desc_get(data.format) };
        ensure!(!descriptor.is_null(), "Unknown software frame format");
        ensure!(
            data.hw_frames_ctx.is_null()
                && unsafe { (*descriptor).flags }
                    & u64::from(ffi::AV_PIX_FMT_FLAG_HWACCEL)
                    == 0,
            "A hardware frame cannot enter the software frame channel"
        );

        Ok(Self(frame))
    }

    pub fn into_frame(self) -> AvFrame {
        self.0
    }
}

/// Where a hardware decoder left a frame's images
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::video) enum HardwareStorage {
    /// CUDA memory from NVDEC
    Cuda,
    /// Vulkan images from Vulkan Video
    Vulkan,
    /// VA-API surfaces
    Vaapi,
}

/// A frame whose images live on the rendering GPU, which it keeps alive
pub(in crate::video) struct HardwareFrame {
    frame: AvFrame,
    storage: HardwareStorage,
    _gpu: Arc<NativeGpu>,
}

impl HardwareFrame {
    pub fn new(frame: AvFrame, storage: HardwareStorage, gpu: Arc<NativeGpu>) -> Self {
        Self {
            frame,
            storage,
            _gpu: gpu,
        }
    }

    pub fn storage(&self) -> HardwareStorage {
        self.storage
    }

    pub fn av_frame(&self) -> &AvFrame {
        &self.frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::ffmpeg::check;

    #[test]
    fn cpu_channel_rejects_hardware_formats() -> Result<()> {
        let frame = AvFrame::allocate()?;

        // SAFETY: This unshared, empty frame owns no buffers and only its format is changed
        unsafe { (*frame.raw()).format = ffi::AV_PIX_FMT_CUDA };
        assert!(CpuFrame::new(frame).is_err());

        Ok(())
    }

    #[test]
    fn cpu_frame_transfers_its_buffer_without_a_gpu_owner() -> Result<()> {
        let frame = AvFrame::allocate()?;

        // SAFETY: This private frame is initialized before allocation and remains exclusively owned
        unsafe {
            (*frame.raw()).format = ffi::AV_PIX_FMT_RGBA;
            (*frame.raw()).width = 16;
            (*frame.raw()).height = 16;
            check(
                ffi::av_frame_get_buffer(frame.raw(), 0),
                "Cannot allocate test pixels",
            )?;
            *(*frame.raw()).data[0] = 173;
        }

        let original = frame.data().data[0] as usize;
        let frame = CpuFrame::new(frame)?;
        let received = std::thread::spawn(move || {
            let frame = frame.into_frame();

            // SAFETY: The receiving thread now exclusively owns the initialized frame
            (frame.data().data[0] as usize, unsafe {
                *frame.data().data[0]
            })
        })
        .join()
        .map_err(|_| anyhow::anyhow!("The frame receiver panicked"))?;

        assert_eq!(received, (original, 173));

        Ok(())
    }
}
