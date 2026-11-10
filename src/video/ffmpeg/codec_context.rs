//! A backend-neutral FFmpeg decoder context with send/receive operations
//!
//! Software threading and hardware device binding are supplied through `CodecSetup`

use std::{any::Any, ptr, ptr::NonNull};

use anyhow::{Context, Result, ensure};

use super::{AvFrame, Packet, Receive, Sent, check, ffi, logging};

/// The largest frame FFmpeg may allocate: 64 Mi pixels, such as 8192 × 8192
const MAX_FRAME_PIXELS: i64 = 64 * 1024 * 1024;

/// How a decoder adapts FFmpeg's generic context before it opens
///
/// The context keeps the setup until it is freed, so the setup may hand FFmpeg
/// pointers into itself for callbacks
pub(in crate::video) trait CodecSetup: Any {
    /// The FFmpeg decoder to open; the default is the stream's own codec
    fn decoder(&self, stream: &ffi::AVStream) -> *const ffi::AVCodec {
        // SAFETY: The stream owns its codec parameters and the decoder description is static
        unsafe { ffi::avcodec_find_decoder((*stream.codecpar).codec_id) }
    }

    /// Adjusts the context after the stream's parameters are copied into it
    fn configure(&mut self, context: &mut ffi::AVCodecContext);
}

/// The stream's own decoder with FFmpeg's default settings
pub(in crate::video) struct FfmpegDefaults;

impl CodecSetup for FfmpegDefaults {
    fn configure(&mut self, _context: &mut ffi::AVCodecContext) {}
}

pub(in crate::video) struct CodecContext {
    handle: NonNull<ffi::AVCodecContext>,
    // FFmpeg may retain addresses inside the setup until avcodec_free_context returns
    _setup: Box<dyn Any>,
}

impl CodecContext {
    pub fn open(stream: &ffi::AVStream, setup: impl CodecSetup) -> Result<Self> {
        logging::install();

        let mut setup = Box::new(setup);
        let decoder = setup.decoder(stream);
        ensure!(!decoder.is_null(), "No decoder supports this codec");

        // SAFETY: The decoder is a static FFmpeg description; Drop owns the new context
        let handle = NonNull::new(unsafe { ffi::avcodec_alloc_context3(decoder) })
            .context("Cannot allocate the FFmpeg codec context")?;
        let mut codec = Self {
            handle,
            _setup: Box::new(()),
        };

        // SAFETY: The context is private and FFmpeg copies the borrowed parameters
        unsafe {
            check(
                ffi::avcodec_parameters_to_context(codec.raw(), stream.codecpar),
                "Cannot copy codec parameters",
            )?;

            let context = codec.handle.as_mut();
            context.pkt_timebase = stream.time_base;
            context.max_pixels = MAX_FRAME_PIXELS;
            setup.configure(context);
        }

        // The boxed setup keeps its address when it moves into the context
        codec._setup = setup;

        // SAFETY: The initialized context and its setup remain owned by this wrapper
        check(
            unsafe { ffi::avcodec_open2(codec.raw(), decoder, ptr::null_mut()) },
            "Cannot initialize the decoder",
        )?;

        Ok(codec)
    }

    pub fn raw(&self) -> *mut ffi::AVCodecContext {
        self.handle.as_ptr()
    }

    pub fn data(&self) -> &ffi::AVCodecContext {
        // SAFETY: This wrapper retains the context for the returned borrow
        unsafe { self.handle.as_ref() }
    }

    pub fn flush(&mut self) {
        // SAFETY: Only the owning thread accesses this context
        unsafe { ffi::avcodec_flush_buffers(self.raw()) };
    }

    pub fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        // SAFETY: The packet is borrowed for the call; NULL drains the codec at EOF
        let result = unsafe {
            ffi::avcodec_send_packet(
                self.raw(),
                packet.map_or(ptr::null(), |packet| packet.raw()),
            )
        };

        match result {
            ffi::FLOWA_AVERROR_AGAIN => Ok(Sent::Again),
            ffi::FLOWA_AVERROR_EOF => Ok(Sent::End),
            _ => {
                check(result, "Cannot send a decoder packet")?;
                Ok(Sent::Accepted)
            }
        }
    }

    pub fn receive(&mut self) -> Result<Receive<AvFrame>> {
        let frame = AvFrame::allocate()?;

        // SAFETY: The codec is exclusively owned and the destination frame is writable
        let result = unsafe { ffi::avcodec_receive_frame(self.raw(), frame.raw()) };

        match result {
            ffi::FLOWA_AVERROR_AGAIN => Ok(Receive::Again),
            ffi::FLOWA_AVERROR_EOF => Ok(Receive::End),
            _ => {
                check(result, "Cannot decode a frame")?;
                Ok(Receive::Item(frame))
            }
        }
    }
}

impl Drop for CodecContext {
    fn drop(&mut self) {
        let mut handle = self.raw();

        // SAFETY: Freeing the context joins codec workers before the setup is dropped
        unsafe { ffi::avcodec_free_context(&mut handle) };
    }
}
