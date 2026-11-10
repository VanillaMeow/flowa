//! FFmpeg frame storage with a separately checked transferable CPU-frame type

use std::ptr::NonNull;

use anyhow::{Context, Result, ensure};

use super::ffi;

pub(in crate::video) struct FrameInfo {
    pub timestamp: i64,
    pub width: i32,
    pub height: i32,
    pub aspect_num: i32,
    pub aspect_den: i32,
    pub rotation: f64,
}

/// Owns one `AVFrame` reference without making hardware frames transferable between threads
pub(in crate::video) struct AvFrame {
    handle: NonNull<ffi::AVFrame>,
}

impl AvFrame {
    pub fn allocate() -> Result<Self> {
        // SAFETY: The newly allocated frame has one owner and is freed by Drop
        let handle = NonNull::new(unsafe { ffi::av_frame_alloc() })
            .context("Cannot allocate a decoded frame")?;

        Ok(Self { handle })
    }

    /// Takes another reference to the same frame data
    pub fn retain(&self) -> Result<Self> {
        // SAFETY: Cloning references the source's buffers without copying or modifying them
        let handle = NonNull::new(unsafe { ffi::av_frame_clone(self.raw()) })
            .context("Cannot retain a decoded frame")?;

        Ok(Self { handle })
    }

    pub fn raw(&self) -> *mut ffi::AVFrame {
        self.handle.as_ptr()
    }

    pub fn data(&self) -> &ffi::AVFrame {
        // SAFETY: This wrapper retains the frame for the returned borrow
        unsafe { self.handle.as_ref() }
    }

    pub fn info(&self) -> FrameInfo {
        let frame = self.data();

        FrameInfo {
            timestamp: frame.best_effort_timestamp,
            width: frame.width,
            height: frame.height,
            aspect_num: frame.sample_aspect_ratio.num,
            aspect_den: frame.sample_aspect_ratio.den,
            rotation: self.rotation().unwrap_or(f64::NAN),
        }
    }

    pub(super) fn rotation(&self) -> Option<f64> {
        // SAFETY: The side data is borrowed from this retained frame
        let matrix = unsafe {
            ffi::av_frame_get_side_data(self.raw(), ffi::AV_FRAME_DATA_DISPLAYMATRIX)
        };

        if matrix.is_null() || unsafe { (*matrix).size } < 9 * size_of::<i32>() {
            return None;
        }

        // SAFETY: DISPLAYMATRIX side data contains an aligned nine-element matrix
        Some(unsafe { ffi::av_display_rotation_get((*matrix).data.cast()) })
    }
}

impl Drop for AvFrame {
    fn drop(&mut self) {
        let mut handle = self.raw();

        // SAFETY: This wrapper owns one frame reference; GPU work retains separate references
        unsafe { ffi::av_frame_free(&mut handle) };
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
            super::super::check(
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
