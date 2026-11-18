//! SDL3 audio output that pulls samples from the playback's audio feed

use std::{ffi::CStr, ptr::NonNull, sync::Arc};

use anyhow::{Result, ensure};

use super::{CHANNELS, RATE, feed::AudioFeed};
use crate::video::ffi::native::{self as native, sdl_error};

/// Keeps SDL subsystem initialization and shutdown on the UI thread
pub(in crate::video) struct AudioRuntime {
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

/// An SDL device stream that pulls its samples from an `AudioFeed`
pub(in crate::video) struct AudioOutput {
    stream: NonNull<native::SDL_AudioStream>,
    /// The callback's reference to the feed, released after the stream is destroyed
    feed: *const AudioFeed,
    pub latency: f64,
    pub name: String,
}

// SAFETY: SDL3 stream functions are thread-safe; the feed pointer is an `Arc` that is
// only read by SDL's callback and released once in Drop
unsafe impl Send for AudioOutput {}

impl AudioOutput {
    /// Opens the default device, paused, playing whatever `feed` provides
    pub fn open(volume: f64, feed: Arc<AudioFeed>) -> Result<Self> {
        let requested = native::SDL_AudioSpec {
            format: native::SDL_AUDIO_F32,
            channels: CHANNELS as i32,
            freq: RATE as i32,
        };
        let feed = Arc::into_raw(feed);

        // SAFETY: SDL copies the format, and the callback's feed lives until Drop
        // destroys the stream
        let stream = unsafe {
            native::SDL_OpenAudioDeviceStream(
                native::SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
                &requested,
                Some(pull),
                feed.cast_mut().cast(),
            )
        };
        let Some(stream) = NonNull::new(stream) else {
            // SAFETY: SDL never received the reference, so it is released here
            drop(unsafe { Arc::from_raw(feed) });
            anyhow::bail!("Cannot open SDL3 audio: {}", sdl_error());
        };

        let output = Self {
            latency: device_latency(stream),
            name: driver_name(),
            stream,
            feed,
        };
        output.volume(volume)?;

        Ok(output)
    }

    pub fn volume(&self, volume: f64) -> Result<()> {
        // SAFETY: The stream remains alive and SDL serializes stream changes
        let success = unsafe {
            native::SDL_SetAudioStreamGain(
                self.stream.as_ptr(),
                (volume / 100.0) as f32,
            )
        };
        check(success, "Cannot set audio volume")
    }

    /// Plays samples faster or slower, which also raises or lowers their pitch
    pub fn set_ratio(&self, ratio: f64) -> Result<()> {
        let ratio = ratio.clamp(0.01, 100.) as f32;

        // SAFETY: The stream remains alive and SDL serializes stream changes
        let success = unsafe {
            native::SDL_SetAudioStreamFrequencyRatio(self.stream.as_ptr(), ratio)
        };
        check(success, "Cannot change the audio speed")
    }

    pub fn pause(&self, paused: bool) -> Result<()> {
        // SAFETY: The stream remains alive and SDL serializes stream changes
        let success = unsafe {
            if paused {
                native::SDL_PauseAudioStreamDevice(self.stream.as_ptr())
            } else {
                native::SDL_ResumeAudioStreamDevice(self.stream.as_ptr())
            }
        };
        check(success, "Cannot change audio pause state")
    }

    /// Pauses and drops what SDL took from the feed but has not played
    pub fn clear(&self) -> Result<()> {
        self.pause(true)?;

        // SAFETY: The stream remains alive and SDL serializes stream changes
        let success = unsafe { native::SDL_ClearAudioStream(self.stream.as_ptr()) };
        check(success, "Cannot clear queued audio")
    }

    /// Bytes SDL took from the feed that the device has not played yet
    pub fn queued_bytes(&self) -> Result<usize> {
        // SAFETY: The stream remains alive and SDL serializes stream changes
        let bytes = unsafe { native::SDL_GetAudioStreamQueued(self.stream.as_ptr()) };
        ensure!(
            bytes >= 0,
            "Cannot read the SDL3 audio queue: {}",
            sdl_error()
        );

        Ok(bytes as usize)
    }
}

/// SDL's request for more samples, answered from the feed
unsafe extern "C" fn pull(
    feed: *mut std::ffi::c_void,
    stream: *mut native::SDL_AudioStream,
    additional: std::ffi::c_int,
    _total: std::ffi::c_int,
) {
    let frames =
        usize::try_from(additional).unwrap_or(0) / (CHANNELS * size_of::<f32>());

    if frames == 0 {
        return;
    }

    // SAFETY: The output keeps this feed alive until the stream is destroyed
    let feed = unsafe { &*feed.cast_const().cast::<AudioFeed>() };

    // A panic must not unwind into SDL's audio thread
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        feed.fill(frames, |samples| {
            let Ok(bytes) = i32::try_from(size_of_val(samples)) else {
                return;
            };

            // SAFETY: SDL copies the samples before returning
            unsafe {
                native::SDL_PutAudioStreamData(stream, samples.as_ptr().cast(), bytes);
            }
        });
    }));
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        // SAFETY: Destroying the stream waits for a running callback, so the feed's
        // reference can be released afterward
        unsafe {
            native::SDL_DestroyAudioStream(self.stream.as_ptr());
            drop(Arc::from_raw(self.feed));
        }
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
