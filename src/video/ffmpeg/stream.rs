//! Immutable, independently owned stream descriptions shared with codec workers

use std::{ptr, ptr::NonNull};

use anyhow::{Context, Result};

use super::{check, ffi};
use crate::video::AudioInfo;

/// Timing, orientation, and codec facts about the playing video and its audio
#[derive(Clone, Copy)]
pub(in crate::video) struct StreamInfo {
    pub origin: f64,
    pub duration: f64,
    pub video_time_base: f64,
    pub audio_time_base: f64,
    pub frame_duration: f64,
    pub rotation: f64,
    pub audio: Option<AudioInfo>,
    pub codec: &'static str,
}

pub(in crate::video) struct Stream {
    container: NonNull<ffi::AVFormatContext>,
    video: NonNull<ffi::AVStream>,
    pub info: StreamInfo,
}

// SAFETY: The independent snapshot is immutable and its container performs no I/O
unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}

impl Stream {
    pub fn copy(source: &ffi::AVStream, info: StreamInfo) -> Result<Self> {
        // SAFETY: The new container owns only copied stream metadata
        let container = NonNull::new(unsafe { ffi::avformat_alloc_context() })
            .context("Cannot allocate stream parameters")?;
        let video =
            unsafe { ffi::avformat_new_stream(container.as_ptr(), ptr::null()) };
        let Some(video) = NonNull::new(video) else {
            unsafe { ffi::avformat_free_context(container.as_ptr()) };
            anyhow::bail!("Cannot allocate the video stream snapshot");
        };
        let mut stream = Self {
            container,
            video,
            info,
        };

        // SAFETY: The destination is independent and the source is borrowed through the copy
        unsafe {
            check(
                ffi::avcodec_parameters_copy(
                    stream.video.as_ref().codecpar,
                    source.codecpar,
                ),
                "Cannot copy the video stream parameters",
            )?;
            stream.video.as_mut().time_base = source.time_base;
            stream.video.as_mut().sample_aspect_ratio = source.sample_aspect_ratio;
        }

        Ok(stream)
    }

    pub fn raw(&self) -> *mut ffi::AVStream {
        self.video.as_ptr()
    }

    pub fn data(&self) -> &ffi::AVStream {
        // SAFETY: The immutable stream is retained by this wrapper
        unsafe { self.video.as_ref() }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: This container owns the snapshot and has no active I/O
        unsafe { ffi::avformat_free_context(self.container.as_ptr()) };
    }
}
