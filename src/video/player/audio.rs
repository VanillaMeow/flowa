//! SDL3 output ownership and queue operations, controlled by the Rust playback state

use std::{
    ffi::{CStr, c_void},
    ptr::{self, NonNull},
    sync::Arc,
};

use anyhow::{Result, ensure};

use crate::video::native::{Api, AudioSpec};

pub(super) const RATE: usize = 48_000;
pub(super) const CHANNELS: usize = 2;
pub(super) const BYTES_PER_SECOND: usize = RATE * CHANNELS * size_of::<f32>();
const SDL_INIT_AUDIO: u32 = 0x10;
const SDL_DEFAULT_PLAYBACK: u32 = u32::MAX;
const SDL_F32: i32 = if cfg!(target_endian = "little") {
    0x8120
} else {
    0x9120
};

/// Keeps SDL subsystem initialization and shutdown on the UI thread
pub(super) struct AudioRuntime(Arc<Api>);

impl AudioRuntime {
    pub fn new(api: Arc<Api>) -> Result<Self> {
        ensure!(
            unsafe { (api.sdl_init)(SDL_INIT_AUDIO) },
            "Cannot initialize SDL3 audio: {}",
            error(&api)
        );
        Ok(Self(api))
    }
}

impl Drop for AudioRuntime {
    fn drop(&mut self) {
        // SAFETY: The player joins playback and destroys its audio streams before shutdown
        unsafe { (self.0.sdl_quit)(SDL_INIT_AUDIO) };
    }
}

/// Owned by playback; queue operations are serialized by Shared
pub(super) struct AudioOutput {
    api: Arc<Api>,
    stream: NonNull<c_void>,
    pub latency: f64,
    pub name: String,
}

// SAFETY: SDL3 queue/control functions are thread-safe and every access holds the playback mutex
// SDL subsystem initialization and shutdown belong to the UI's AudioRuntime
unsafe impl Send for AudioOutput {}

impl AudioOutput {
    pub fn open(api: Arc<Api>, volume: f64) -> Result<Self> {
        let requested = AudioSpec {
            format: SDL_F32,
            channels: CHANNELS as i32,
            freq: RATE as i32,
        };
        let stream = unsafe {
            (api.sdl_audio_open)(
                SDL_DEFAULT_PLAYBACK,
                &requested,
                None,
                ptr::null_mut(),
            )
        };
        let Some(stream) = NonNull::new(stream) else {
            anyhow::bail!("Cannot open SDL3 audio: {}", error(&api));
        };

        let mut actual = AudioSpec::default();
        let mut period = 0;
        let device = unsafe { (api.sdl_audio_device)(stream.as_ptr()) };
        let known_format =
            unsafe { (api.sdl_audio_format)(device, &mut actual, &mut period) };
        let latency = if known_format && actual.freq > 0 {
            2.0 * f64::from(period) / f64::from(actual.freq)
        } else {
            0.0
        };
        let driver = unsafe { (api.sdl_audio_driver)() };
        let name = if driver.is_null() {
            "sdl3".into()
        } else {
            // SAFETY: SDL retains the driver name until audio shutdown
            format!(
                "sdl3/{}",
                unsafe { CStr::from_ptr(driver) }.to_string_lossy()
            )
        };
        let output = Self {
            api,
            stream,
            latency,
            name,
        };
        output.volume(volume)?;
        Ok(output)
    }

    pub fn volume(&self, volume: f64) -> Result<()> {
        self.check(
            unsafe {
                (self.api.sdl_audio_gain)(self.stream.as_ptr(), (volume / 100.0) as f32)
            },
            "Cannot set audio volume",
        )
    }

    pub fn pause(&self, paused: bool) -> Result<()> {
        // SAFETY: The stream remains alive and queue/control calls are mutex-serialized
        let success = unsafe {
            if paused {
                (self.api.sdl_audio_pause)(self.stream.as_ptr())
            } else {
                (self.api.sdl_audio_resume)(self.stream.as_ptr())
            }
        };
        self.check(success, "Cannot change audio pause state")
    }

    pub fn clear(&self) -> Result<()> {
        self.pause(true)?;
        self.check(
            unsafe { (self.api.sdl_audio_clear)(self.stream.as_ptr()) },
            "Cannot clear queued audio",
        )
    }

    pub fn queued_bytes(&self) -> Result<usize> {
        let bytes = unsafe { (self.api.sdl_audio_queued)(self.stream.as_ptr()) };
        ensure!(
            bytes >= 0,
            "Cannot read the SDL3 audio queue: {}",
            error(&self.api)
        );
        Ok(bytes as usize)
    }

    pub fn put(&self, samples: &[f32]) -> Result<()> {
        let bytes = i32::try_from(size_of_val(samples))?;

        // SAFETY: SDL copies the borrowed PCM data before returning
        self.check(
            unsafe {
                (self.api.sdl_audio_put)(
                    self.stream.as_ptr(),
                    samples.as_ptr().cast(),
                    bytes,
                )
            },
            "Cannot queue SDL3 audio",
        )
    }

    pub fn flush(&self) -> Result<()> {
        self.check(
            unsafe { (self.api.sdl_audio_flush)(self.stream.as_ptr()) },
            "Cannot flush SDL3 audio",
        )
    }

    fn check(&self, success: bool, message: &str) -> Result<()> {
        ensure!(success, "{message}: {}", error(&self.api));
        Ok(())
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        // SAFETY: The engine has joined the media worker before releasing its stream
        unsafe {
            (self.api.sdl_audio_destroy)(self.stream.as_ptr());
        }
    }
}

fn error(api: &Api) -> String {
    // SAFETY: SDL returns a thread-local NUL-terminated message valid through the next SDL call
    unsafe { CStr::from_ptr((api.sdl_error)()) }
        .to_string_lossy()
        .into_owned()
}
