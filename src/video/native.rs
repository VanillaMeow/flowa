//! Declarations for the Vulkan/libplacebo shim and SDL3 audio, generated from their headers

use std::{
    ffi::{CStr, c_char, c_int},
    fmt,
};

mod bindings {
    #![allow(
        dead_code,
        non_camel_case_types,
        non_snake_case,
        non_upper_case_globals
    )]
    #![allow(clippy::all)]

    use ash::vk;

    pub(in crate::video) use crate::video::ffmpeg::{AVBufferRef, AVFrame, AVStream};

    // Ash's handle and struct definitions share the C ABI of the Vulkan headers
    pub type VkInstance = vk::Instance;
    pub type VkPhysicalDevice = vk::PhysicalDevice;
    pub type VkDevice = vk::Device;
    pub type VkImage = vk::Image;
    pub type VkSemaphore = vk::Semaphore;
    pub type VkPhysicalDeviceFeatures2 = vk::PhysicalDeviceFeatures2<'static>;
    pub type PFN_vkGetInstanceProcAddr = vk::PFN_vkGetInstanceProcAddr;

    include!(concat!(env!("OUT_DIR"), "/native.rs"));
}

pub(super) use bindings::*;

/// The header defines this as a cast expression, which bindgen cannot translate
pub(super) const SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK: SDL_AudioDeviceID = 0xFFFF_FFFF;

/// A negative status from the shim with the message it wrote
#[derive(Debug)]
pub(super) struct NativeError {
    pub status: c_int,
    message: String,
}

impl fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for NativeError {}

/// Runs a shim call that reports failures as a negative status plus a message
///
/// The message buffer lives only for the duration of the call
pub(super) fn call(
    call: impl FnOnce(*mut c_char, usize) -> c_int,
) -> Result<c_int, NativeError> {
    let mut error = [0; 768];
    let status = call(error.as_mut_ptr(), error.len());

    if status < 0 {
        return Err(NativeError {
            status,
            message: error_message(&error),
        });
    }

    Ok(status)
}

fn error_message(error: &[c_char]) -> String {
    let message = string(error);
    if message.is_empty() {
        "Native video operation failed".into()
    } else {
        message
    }
}

/// Copies one bounded, NUL-terminated string owned by the native backend
pub(super) fn string(value: &[c_char]) -> String {
    let bytes: Vec<_> = value.iter().map(|&byte| byte as u8).collect();

    CStr::from_bytes_until_nul(&bytes)
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Reads SDL's thread-local error message for the most recent failed call
pub(super) fn sdl_error() -> String {
    // SAFETY: SDL returns a NUL-terminated message valid until its next call on this thread
    unsafe { CStr::from_ptr(SDL_GetError()) }
        .to_string_lossy()
        .into_owned()
}
