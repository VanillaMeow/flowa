//! SDL3 output ownership and queue operations, controlled by the Rust playback state

use std::{
    ffi::CStr,
    ptr::{self, NonNull},
};

use anyhow::{Result, ensure};

use crate::video::native::{self, sdl_error};

pub(super) const RATE: usize = 48_000;
pub(super) const CHANNELS: usize = 2;
pub(super) const BYTES_PER_SECOND: usize = RATE * CHANNELS * size_of::<f32>();

/// Keeps SDL subsystem initialization and shutdown on the UI thread
pub(super) struct AudioRuntime {
    _private: (),
}

impl AudioRuntime {
    pub fn new() -> Result<Self> {
        // SAFETY: Initialization takes plain flags and is balanced by Drop
        let initialized = unsafe { native::SDL_InitSubSystem(native::SDL_INIT_AUDIO) };
        ensure!(initialized, "Cannot initialize SDL3 audio: {}", sdl_error());

        Ok(Self { _private: () })
    }
}

impl Drop for AudioRuntime {
    fn drop(&mut self) {
        // SAFETY: The player joins playback and destroys its audio streams before shutdown
        unsafe { native::SDL_QuitSubSystem(native::SDL_INIT_AUDIO) };
    }
}

/// Owned by playback; queue operations are serialized by Shared
pub(super) struct AudioOutput {
    stream: NonNull<native::SDL_AudioStream>,
    pub latency: f64,
    pub name: String,
}

// SAFETY: SDL3 queue/control functions are thread-safe and every access holds the playback mutex
// SDL subsystem initialization and shutdown belong to the UI's AudioRuntime
unsafe impl Send for AudioOutput {}

impl AudioOutput {
    pub fn open(volume: f64) -> Result<Self> {
        let requested = native::SDL_AudioSpec {
            format: native::SDL_AUDIO_F32,
            channels: CHANNELS as i32,
            freq: RATE as i32,
        };

        // SAFETY: SDL copies the requested format and owns the returned stream until destroyed
        let stream = unsafe {
            native::SDL_OpenAudioDeviceStream(
                native::SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
                &requested,
                None,
                ptr::null_mut(),
            )
        };
        let Some(stream) = NonNull::new(stream) else {
            anyhow::bail!("Cannot open SDL3 audio: {}", sdl_error());
        };

        let output = Self {
            latency: device_latency(stream),
            name: driver_name(),
            stream,
        };
        output.volume(volume)?;

        Ok(output)
    }

    pub fn volume(&self, volume: f64) -> Result<()> {
        // SAFETY: The stream remains alive and queue/control calls are mutex-serialized
        let success = unsafe {
            native::SDL_SetAudioStreamGain(
                self.stream.as_ptr(),
                (volume / 100.0) as f32,
            )
        };
        check(success, "Cannot set audio volume")
    }

    pub fn pause(&self, paused: bool) -> Result<()> {
        // SAFETY: The stream remains alive and queue/control calls are mutex-serialized
        let success = unsafe {
            if paused {
                native::SDL_PauseAudioStreamDevice(self.stream.as_ptr())
            } else {
                native::SDL_ResumeAudioStreamDevice(self.stream.as_ptr())
            }
        };
        check(success, "Cannot change audio pause state")
    }

    pub fn clear(&self) -> Result<()> {
        self.pause(true)?;

        // SAFETY: The stream remains alive and queue/control calls are mutex-serialized
        let success = unsafe { native::SDL_ClearAudioStream(self.stream.as_ptr()) };
        check(success, "Cannot clear queued audio")
    }

    pub fn queued_bytes(&self) -> Result<usize> {
        // SAFETY: The stream remains alive and queue/control calls are mutex-serialized
        let bytes = unsafe { native::SDL_GetAudioStreamQueued(self.stream.as_ptr()) };
        ensure!(
            bytes >= 0,
            "Cannot read the SDL3 audio queue: {}",
            sdl_error()
        );

        Ok(bytes as usize)
    }

    pub fn put(&self, samples: &[f32]) -> Result<()> {
        let bytes = i32::try_from(size_of_val(samples))?;

        // SAFETY: SDL copies the borrowed PCM data before returning
        let success = unsafe {
            native::SDL_PutAudioStreamData(
                self.stream.as_ptr(),
                samples.as_ptr().cast(),
                bytes,
            )
        };
        check(success, "Cannot queue SDL3 audio")
    }

    pub fn flush(&self) -> Result<()> {
        // SAFETY: The stream remains alive and queue/control calls are mutex-serialized
        let success = unsafe { native::SDL_FlushAudioStream(self.stream.as_ptr()) };
        check(success, "Cannot flush SDL3 audio")
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        // SAFETY: The engine has joined the media worker before releasing its stream
        unsafe { native::SDL_DestroyAudioStream(self.stream.as_ptr()) };
    }
}

/// Estimates output latency as two device periods, or zero when SDL cannot report it
fn device_latency(stream: NonNull<native::SDL_AudioStream>) -> f64 {
    let mut actual = native::SDL_AudioSpec {
        format: native::SDL_AUDIO_UNKNOWN,
        channels: 0,
        freq: 0,
    };
    let mut period = 0;

    // SAFETY: The stream is open and SDL writes only into the provided outputs
    let known = unsafe {
        let device = native::SDL_GetAudioStreamDevice(stream.as_ptr());
        native::SDL_GetAudioDeviceFormat(device, &mut actual, &mut period)
    };

    if known && actual.freq > 0 {
        2.0 * f64::from(period) / f64::from(actual.freq)
    } else {
        0.0
    }
}

fn driver_name() -> String {
    // SAFETY: SDL retains the driver name until audio shutdown
    let driver = unsafe { native::SDL_GetCurrentAudioDriver() };

    if driver.is_null() {
        return "sdl3".into();
    }

    // SAFETY: A non-null driver name is a NUL-terminated string owned by SDL
    let name = unsafe { CStr::from_ptr(driver) }.to_string_lossy();
    format!("sdl3/{name}")
}

fn check(success: bool, message: &str) -> Result<()> {
    ensure!(success, "{message}: {}", sdl_error());

    Ok(())
}
