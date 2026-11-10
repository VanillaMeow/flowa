//! Declarations for libplacebo, its shim, and SDL3 audio, generated from their headers

use std::ffi::CStr;

mod bindings {
    #![allow(
        dead_code,
        non_camel_case_types,
        non_snake_case,
        non_upper_case_globals
    )]
    #![allow(clippy::all)]

    use super::super::{
        ffmpeg::{AVFrame, AVStream},
        vulkan::*,
    };

    include!(concat!(env!("OUT_DIR"), "/native.rs"));
}

pub(in crate::video) use bindings::*;

/// The header defines this as a cast expression, which bindgen cannot translate
pub(in crate::video) const SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK: SDL_AudioDeviceID =
    0xFFFF_FFFF;

/// Reads SDL's thread-local error message for the most recent failed call
pub(in crate::video) fn sdl_error() -> String {
    // SAFETY: SDL returns a NUL-terminated message valid until its next call on this thread
    unsafe { CStr::from_ptr(SDL_GetError()) }
        .to_string_lossy()
        .into_owned()
}
