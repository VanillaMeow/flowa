//! An FFmpeg format context with owned interruption storage

use std::{ffi::c_void, path::Path, ptr};

use anyhow::{Result, ensure};

use crate::utils::units::MIB;

use super::{Packet, check, ffi, path_string};

struct Interrupt<'a> {
    check: Box<dyn Fn() -> bool + Send + Sync + 'a>,
}

unsafe extern "C" fn interrupted(data: *mut c_void) -> i32 {
    // SAFETY: Input retains this boxed callback until avformat_close_input returns
    let interrupt = unsafe { &*data.cast::<Interrupt<'_>>() };

    i32::from((interrupt.check)())
}

/// How much of the file FFmpeg reads to find the streams
pub(in crate::video) enum Probe {
    Playback,
    Poster,
}

pub(in crate::video) struct Input<'a> {
    handle: *mut ffi::AVFormatContext,
    _interrupt: Box<Interrupt<'a>>,
}

impl<'a> Input<'a> {
    pub fn open(
        path: &Path,
        cancel: impl Fn() -> bool + Send + Sync + 'a,
        probe: Probe,
    ) -> Result<Self> {
        let path = path_string(path)?;
        let mut interrupt = Box::new(Interrupt {
            check: Box::new(cancel),
        });

        // SAFETY: The new context has a stable, retained interruption callback
        let handle = unsafe { ffi::avformat_alloc_context() };
        ensure!(
            !handle.is_null(),
            "Cannot allocate the FFmpeg media context"
        );

        unsafe {
            (*handle).interrupt_callback = ffi::AVIOInterruptCB {
                callback: Some(interrupted),
                opaque: std::ptr::from_mut(&mut *interrupt).cast(),
            };

            if matches!(probe, Probe::Poster) {
                (*handle).probesize = (2 * MIB) as i64;
                (*handle).max_analyze_duration = i64::from(ffi::AV_TIME_BASE);
            }
        }

        let mut input = Self {
            handle,
            _interrupt: interrupt,
        };

        // SAFETY: FFmpeg may replace or free the context; Drop always sees the updated pointer
        unsafe {
            check(
                ffi::avformat_open_input(
                    &mut input.handle,
                    path.as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                ),
                "Cannot open media",
            )?;
            check(
                ffi::avformat_find_stream_info(input.handle, ptr::null_mut()),
                "Cannot probe media streams",
            )?;
        }

        Ok(input)
    }

    pub fn raw(&self) -> *mut ffi::AVFormatContext {
        self.handle
    }

    pub fn data(&self) -> &ffi::AVFormatContext {
        // SAFETY: Successful opening guarantees a live context retained by this wrapper
        unsafe { &*self.handle }
    }

    pub fn stream(&self, index: usize) -> &ffi::AVStream {
        assert!(
            index < self.data().nb_streams as usize,
            "Invalid stream index"
        );

        // SAFETY: The context owns the validated stream array and the selected stream
        unsafe { &**self.data().streams.add(index) }
    }

    pub fn best_stream(&self, kind: ffi::AVMediaType) -> Result<usize> {
        // SAFETY: Stream discovery only borrows this live demuxer
        let index = unsafe {
            ffi::av_find_best_stream(self.handle, kind, -1, -1, ptr::null_mut(), 0)
        };

        Ok(check(index, "Cannot find a media stream")? as usize)
    }

    pub fn read(&mut self) -> Result<Option<Packet>> {
        let mut packet = Packet::allocate()?;

        // SAFETY: The demuxer has one owner and the packet is writable
        let result = unsafe { ffi::av_read_frame(self.handle, packet.raw()) };
        if result == ffi::FLOWA_AVERROR_EOF {
            return Ok(None);
        }
        check(result, "Cannot read media")?;

        packet.bytes = usize::try_from(unsafe { (*packet.raw()).size })?;

        Ok(Some(packet))
    }

    /// Clears the error an interrupted read leaves behind, so the next read can proceed
    pub fn clear_interrupted_io(&mut self) {
        // FFmpeg retains AVERROR_EXIT in AVIO even after a successful buffered seek
        // SAFETY: Only the demux worker accesses this context and its I/O state
        unsafe {
            let io = (*self.handle).pb;

            if !io.is_null() && (*io).error == ffi::FLOWA_AVERROR_EXIT {
                (*io).error = 0;
                (*io).eof_reached = 0;
            }
        }
    }
}

impl Drop for Input<'_> {
    fn drop(&mut self) {
        // SAFETY: The interruption allocation remains alive until closing I/O completes
        unsafe { ffi::avformat_close_input(&mut self.handle) };
    }
}
