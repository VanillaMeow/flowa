//! Declarations for the Vulkan/libplacebo shim and SDL3 audio, generated from their headers

use std::ffi::{CStr, c_char, c_int};

use anyhow::{Result, ensure};

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

/// Preserves the native error while keeping all temporary C buffers inside the call
pub(super) fn check(call: impl FnOnce(*mut c_char, usize) -> c_int) -> Result<c_int> {
    let mut error = [0; 768];
    let result = call(error.as_mut_ptr(), error.len());
    ensure!(result >= 0, "{}", error_message(&error));
    Ok(result)
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
