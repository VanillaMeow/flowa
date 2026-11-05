//! Typed FFmpeg/libplacebo and SDL declarations linked by Cargo

use std::{
    ffi::{CStr, c_char, c_int, c_void},
    sync::Arc,
};

use anyhow::{Result, ensure};
use ash::vk;

pub(super) type Handle = *mut c_void;
pub(super) type Interrupt = Option<unsafe extern "C" fn(*mut c_void) -> c_int>;
pub(super) type Logger = Option<unsafe extern "C" fn(c_int, *const c_char)>;
pub(super) type AudioCallback =
    Option<unsafe extern "C" fn(*mut c_void, Handle, c_int, c_int)>;

#[repr(C)]
pub(super) struct GpuInit {
    pub instance: vk::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub get_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    pub extensions: *const *const c_char,
    pub num_extensions: c_int,
    pub features: *const vk::PhysicalDeviceFeatures2<'static>,
    pub log: Logger,
}

#[repr(C)]
pub(super) struct GpuInfo {
    pub device: vk::Device,
    pub graphics_family: u32,
    pub extensions: *const *const c_char,
    pub num_extensions: c_int,
    pub features: *const vk::PhysicalDeviceFeatures2<'static>,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(super) struct StreamInfo {
    pub origin: f64,
    pub duration: f64,
    pub video_time_base: f64,
    pub audio_time_base: f64,
    pub frame_duration: f64,
    pub rotation: f64,
    pub has_audio: c_int,
    pub codec: [c_char; 64],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(super) struct FrameInfo {
    pub timestamp: i64,
    pub width: c_int,
    pub height: c_int,
    pub aspect_num: c_int,
    pub aspect_den: c_int,
    pub rotation: f64,
}

#[repr(C)]
pub(super) struct AudioBlock {
    pub samples: *mut f32,
    pub frames: c_int,
    pub timestamp: i64,
    pub resample_delay: f64,
}

#[derive(Default)]
#[repr(C)]
pub(super) struct AudioSpec {
    pub format: c_int,
    pub channels: c_int,
    pub freq: c_int,
}

#[repr(C)]
pub(super) struct Target {
    pub image: vk::Image,
    pub semaphore: vk::Semaphore,
    pub wait_value: u64,
    pub signal_value: u64,
    pub width: u32,
    pub height: u32,
    pub slot: u32,
}

// Keep function pointers and linked declarations on the same typed API
macro_rules! define_api {
    ($(fn $name:ident($($argument:ident: $kind:ty),*) -> $result:ty = $symbol:literal;)*) => {
        pub(super) struct Api {
            $(pub $name: unsafe extern "C" fn($($kind),*) -> $result,)*
        }

        mod linked {
            use super::*;

            unsafe extern "C" {
                $(#[link_name = $symbol]
                pub fn $name($($argument: $kind),*) -> $result;)*
            }
        }

        impl Api {
            pub fn load() -> Result<Arc<Self>> {
                let api = Self { $($name: linked::$name,)* };

                // SAFETY: The version query takes no arguments and owns no resources
                ensure!(unsafe { (api.video_abi_version)() } == 3,
                    "Unsupported Flowa FFmpeg shim ABI; rebuild Flowa with Cargo");
                Ok(Arc::new(api))
            }
        }
    };
}

define_api! {
    fn video_abi_version() -> u32 = "flowa_video_abi_version";
    fn gpu_create(init: *const GpuInit, error: *mut c_char, error_size: usize) -> Handle = "flowa_gpu_create";
    fn gpu_destroy(gpu: Handle) -> () = "flowa_gpu_destroy";
    fn gpu_get_info(gpu: Handle, info: *mut GpuInfo) -> () = "flowa_gpu_get_info";
    fn gpu_retire_targets(gpu: Handle) -> () = "flowa_gpu_retire_targets";
    fn gpu_reset_frames(gpu: Handle) -> () = "flowa_gpu_reset_frames";
    fn gpu_render(gpu: Handle, frame: Handle, stream: Handle, target: *const Target, error: *mut c_char, size: usize) -> c_int = "flowa_gpu_render";
    fn media_open(path: *const c_char, interrupt: Interrupt, data: *mut c_void, info: *mut StreamInfo, error: *mut c_char, size: usize) -> Handle = "flowa_media_open";
    fn media_close(media: Handle) -> () = "flowa_media_close";
    fn media_copy_stream(media: Handle) -> Handle = "flowa_media_copy_stream";
    fn stream_free(stream: Handle) -> () = "flowa_stream_free";
    fn media_read(media: Handle, packet: *mut Handle, kind: *mut c_int, bytes: *mut c_int, error: *mut c_char, size: usize) -> c_int = "flowa_media_read";
    fn media_seek(media: Handle, timestamp: i64, error: *mut c_char, size: usize) -> c_int = "flowa_media_seek";
    fn packet_clone(packet: Handle) -> Handle = "flowa_packet_clone";
    fn packet_free(packet: Handle) -> () = "flowa_packet_free";
    fn decoder_create(gpu: Handle, stream: Handle, backend: c_int, error: *mut c_char, size: usize) -> Handle = "flowa_decoder_create";
    fn decoder_free(decoder: Handle) -> () = "flowa_decoder_free";
    fn decoder_flush(decoder: Handle) -> () = "flowa_decoder_flush";
    fn decoder_send(decoder: Handle, packet: Handle, error: *mut c_char, size: usize) -> c_int = "flowa_decoder_send";
    fn decoder_receive(decoder: Handle, frame: *mut Handle, error: *mut c_char, size: usize) -> c_int = "flowa_decoder_receive";
    fn frame_free(frame: Handle) -> () = "flowa_frame_free";
    fn frame_get_info(frame: Handle, info: *mut FrameInfo) -> () = "flowa_frame_get_info";
    fn video_thumbnail(path: *const c_char, interrupt: Interrupt, data: *mut c_void, pixels: *mut u8, max_width: c_int, max_height: c_int, width: *mut c_int, height: *mut c_int, rotation: *mut f64, error: *mut c_char, size: usize) -> c_int = "flowa_video_thumbnail";
    fn media_audio_send(media: Handle, packet: Handle, error: *mut c_char, size: usize) -> c_int = "flowa_media_audio_send";
    fn media_audio_receive(media: Handle, block: *mut AudioBlock, error: *mut c_char, size: usize) -> c_int = "flowa_media_audio_receive";
    fn audio_block_free(samples: *mut f32) -> () = "flowa_audio_block_free";

    fn sdl_init(flags: u32) -> bool = "SDL_InitSubSystem";
    fn sdl_quit(flags: u32) -> () = "SDL_QuitSubSystem";
    fn sdl_error() -> *const c_char = "SDL_GetError";
    fn sdl_audio_driver() -> *const c_char = "SDL_GetCurrentAudioDriver";
    fn sdl_audio_open(device: u32, spec: *const AudioSpec, callback: AudioCallback, data: *mut c_void) -> Handle = "SDL_OpenAudioDeviceStream";
    fn sdl_audio_destroy(stream: Handle) -> () = "SDL_DestroyAudioStream";
    fn sdl_audio_device(stream: Handle) -> u32 = "SDL_GetAudioStreamDevice";
    fn sdl_audio_format(device: u32, spec: *mut AudioSpec, period: *mut c_int) -> bool = "SDL_GetAudioDeviceFormat";
    fn sdl_audio_gain(stream: Handle, gain: f32) -> bool = "SDL_SetAudioStreamGain";
    fn sdl_audio_pause(stream: Handle) -> bool = "SDL_PauseAudioStreamDevice";
    fn sdl_audio_resume(stream: Handle) -> bool = "SDL_ResumeAudioStreamDevice";
    fn sdl_audio_clear(stream: Handle) -> bool = "SDL_ClearAudioStream";
    fn sdl_audio_flush(stream: Handle) -> bool = "SDL_FlushAudioStream";
    fn sdl_audio_queued(stream: Handle) -> c_int = "SDL_GetAudioStreamQueued";
    fn sdl_audio_put(stream: Handle, samples: *const c_void, bytes: c_int) -> bool = "SDL_PutAudioStreamData";
}

/// Preserves the native error while keeping all temporary C buffers inside the call
pub(super) fn check(call: impl FnOnce(*mut c_char, usize) -> c_int) -> Result<c_int> {
    let mut error = [0; 768];
    let result = call(error.as_mut_ptr(), error.len());
    ensure!(result >= 0, "{}", error_message(&error));
    Ok(result)
}

pub(super) fn create(
    call: impl FnOnce(*mut c_char, usize) -> Handle,
) -> Result<std::ptr::NonNull<c_void>> {
    let mut error = [0; 768];
    let pointer = call(error.as_mut_ptr(), error.len());
    std::ptr::NonNull::new(pointer)
        .ok_or_else(|| anyhow::anyhow!(error_message(&error)))
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
