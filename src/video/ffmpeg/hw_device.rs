//! An owned reference to one of FFmpeg's hardware device contexts

use std::ptr::NonNull;

use anyhow::{Context, Result};

use super::ffi;

pub(in crate::video) struct HwDevice(NonNull<ffi::AVBufferRef>);

impl HwDevice {
    /// Takes ownership of one initialized FFmpeg hardware-device reference
    ///
    /// # Safety
    /// The pointer must be null or a reference the caller owns. The caller retains
    /// external resources until all dependent codecs and frames are released
    pub unsafe fn from_raw(pointer: *mut ffi::AVBufferRef) -> Result<Self> {
        Ok(Self(
            NonNull::new(pointer)
                .context("The hardware decoder device is unavailable")?,
        ))
    }

    pub fn raw(&self) -> *mut ffi::AVBufferRef {
        self.0.as_ptr()
    }

    /// Gives the reference to an FFmpeg object that releases it
    pub fn into_raw(self) -> *mut ffi::AVBufferRef {
        let pointer = self.0.as_ptr();
        std::mem::forget(self);

        pointer
    }

    pub fn kind(&self) -> ffi::AVHWDeviceType {
        // SAFETY: Construction requires a reference containing an initialized AVHWDeviceContext
        unsafe { (*self.0.as_ref().data.cast::<ffi::AVHWDeviceContext>()).type_ }
    }
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        let mut pointer = self.0.as_ptr();

        // SAFETY: This wrapper owns one FFmpeg device reference
        unsafe { ffi::av_buffer_unref(&mut pointer) };
    }
}
