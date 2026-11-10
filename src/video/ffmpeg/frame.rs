//! Owned FFmpeg frame references and the properties playback reads from them

use std::ptr::NonNull;

use anyhow::{Context, Result};

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

    pub fn rotation(&self) -> Option<f64> {
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
