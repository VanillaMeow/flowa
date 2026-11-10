//! Codec construction and send/receive operations with stable callback storage

use std::{ffi::c_void, ptr, ptr::NonNull};

use anyhow::{Context, Result, ensure};

use super::{AvFrame, Packet, Receive, Sent, Stream, check, ffi};

pub(in crate::video) struct DeviceRef(NonNull<ffi::AVBufferRef>);

impl DeviceRef {
    /// Takes ownership of one initialized FFmpeg hardware-device reference
    /// The caller retains external resources until all dependent codecs and frames are released
    pub unsafe fn from_raw(pointer: *mut ffi::AVBufferRef) -> Result<Self> {
        Ok(Self(
            NonNull::new(pointer)
                .context("The hardware decoder device is unavailable")?,
        ))
    }

    pub fn raw(&self) -> *mut ffi::AVBufferRef {
        self.0.as_ptr()
    }

    fn into_raw(self) -> *mut ffi::AVBufferRef {
        let pointer = self.0.as_ptr();
        std::mem::forget(self);

        pointer
    }

    fn kind(&self) -> ffi::AVHWDeviceType {
        // SAFETY: Construction requires a reference containing an initialized AVHWDeviceContext
        unsafe { (*self.0.as_ref().data.cast::<ffi::AVHWDeviceContext>()).type_ }
    }
}

impl Drop for DeviceRef {
    fn drop(&mut self) {
        let mut pointer = self.0.as_ptr();

        // SAFETY: This wrapper owns one FFmpeg device reference
        unsafe { ffi::av_buffer_unref(&mut pointer) };
    }
}

/// The largest frame FFmpeg may allocate: 64 Mi pixels, such as 8192 × 8192
const MAX_FRAME_PIXELS: i64 = 64 * 1024 * 1024;

/// Decoder threading for video codecs; audio codecs keep FFmpeg's default
#[derive(Clone, Copy)]
pub(in crate::video) enum Threads {
    /// Lets FFmpeg pick frame and slice threads for sustained playback
    Automatic,
    /// Avoids thread startup and frame delay when decoding a single poster frame
    Single,
}

pub(in crate::video) struct Codec {
    handle: NonNull<ffi::AVCodecContext>,
    // FFmpeg retains this address through get_format, including while the Rust owner moves
    format: Option<Box<ffi::AVPixelFormat>>,
}

unsafe extern "C" fn hardware_format(
    context: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    // SAFETY: Codec retains the boxed selection until avcodec_free_context completes
    let selected = unsafe { *(*context).opaque.cast::<ffi::AVPixelFormat>() };
    let mut cursor = formats;

    // SAFETY: FFmpeg passes an array terminated by AV_PIX_FMT_NONE
    while unsafe { *cursor } != ffi::AV_PIX_FMT_NONE {
        if unsafe { *cursor } == selected {
            return selected;
        }

        cursor = unsafe { cursor.add(1) };
    }

    // Returning NONE prevents an unexpected software format on a hardware path
    ffi::AV_PIX_FMT_NONE
}

impl Codec {
    pub fn software(stream: &Stream) -> Result<Self> {
        Self::software_stream(stream.data(), Threads::Automatic)
    }

    pub(super) fn software_stream(
        stream: &ffi::AVStream,
        threads: Threads,
    ) -> Result<Self> {
        // SAFETY: The source owns its codec parameters and the decoder description is static
        let decoder = unsafe { ffi::avcodec_find_decoder((*stream.codecpar).codec_id) };
        let mut codec = Self::allocate(decoder)?;
        codec.configure(stream)?;

        // SAFETY: The context is private and has not been opened
        unsafe {
            let context = codec.handle.as_mut();

            if (*stream.codecpar).codec_type == ffi::AVMEDIA_TYPE_VIDEO {
                // A zero thread count asks FFmpeg to size the pool from the CPU
                context.thread_count = match threads {
                    Threads::Automatic => 0,
                    Threads::Single => 1,
                };
                context.thread_type =
                    (ffi::FF_THREAD_FRAME | ffi::FF_THREAD_SLICE) as i32;
            }
        }

        codec.open(decoder)?;

        Ok(codec)
    }

    pub fn hardware(stream: &Stream, device: DeviceRef) -> Result<Self> {
        let (decoder, format) = Self::find_hardware(stream.data(), device.kind())?;
        let mut codec = Self::allocate(decoder)?;
        codec.configure(stream.data())?;
        let mut selected = Box::new(format);
        let callback_data = std::ptr::from_mut(&mut *selected).cast::<c_void>();
        codec.format = Some(selected);

        // SAFETY: The private context takes the device reference and borrows stable boxed storage
        unsafe {
            let context = codec.handle.as_mut();
            context.hw_device_ctx = device.into_raw();
            context.get_format = Some(hardware_format);
            context.opaque = callback_data;
            context.thread_count = 1;
            context.thread_type = 0;
        }

        codec.open(decoder)?;

        Ok(codec)
    }

    fn find_hardware(
        stream: &ffi::AVStream,
        device_type: ffi::AVHWDeviceType,
    ) -> Result<(*const ffi::AVCodec, ffi::AVPixelFormat)> {
        let mut iterator = ptr::null_mut();

        // SAFETY: FFmpeg returns static descriptions while the source owns its codec parameters
        unsafe {
            loop {
                let decoder = ffi::av_codec_iterate(&mut iterator);
                if decoder.is_null() {
                    anyhow::bail!("The hardware backend does not support this codec");
                }
                if (*decoder).id != (*stream.codecpar).codec_id
                    || ffi::av_codec_is_decoder(decoder) == 0
                {
                    continue;
                }

                let mut index = 0;

                loop {
                    let configuration = ffi::avcodec_get_hw_config(decoder, index);
                    if configuration.is_null() {
                        break;
                    }
                    if (*configuration).device_type == device_type
                        && (*configuration).methods
                            & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32
                            != 0
                    {
                        return Ok((decoder, (*configuration).pix_fmt));
                    }

                    index += 1;
                }
            }
        }
    }

    fn allocate(decoder: *const ffi::AVCodec) -> Result<Self> {
        ensure!(!decoder.is_null(), "No decoder supports this codec");

        // SAFETY: The decoder is a static FFmpeg description; Drop owns the new context
        let handle = NonNull::new(unsafe { ffi::avcodec_alloc_context3(decoder) })
            .context("Cannot allocate the FFmpeg codec context")?;

        Ok(Self {
            handle,
            format: None,
        })
    }

    fn configure(&mut self, stream: &ffi::AVStream) -> Result<()> {
        // SAFETY: The context is private and FFmpeg copies the borrowed parameters
        unsafe {
            check(
                ffi::avcodec_parameters_to_context(self.raw(), stream.codecpar),
                "Cannot copy codec parameters",
            )?;
            self.handle.as_mut().pkt_timebase = stream.time_base;
            self.handle.as_mut().max_pixels = MAX_FRAME_PIXELS;
        }

        Ok(())
    }

    fn open(&mut self, decoder: *const ffi::AVCodec) -> Result<()> {
        // SAFETY: The initialized context and any callback storage remain owned by this wrapper
        check(
            unsafe { ffi::avcodec_open2(self.raw(), decoder, ptr::null_mut()) },
            "Cannot initialize the decoder",
        )?;

        Ok(())
    }

    pub(super) fn raw(&self) -> *mut ffi::AVCodecContext {
        self.handle.as_ptr()
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
            ffi::FLOWA_AVERROR_AGAIN => return Ok(Receive::Again),
            ffi::FLOWA_AVERROR_EOF => return Ok(Receive::End),
            _ => check(result, "Cannot decode a frame")?,
        };

        if let Some(format) = &self.format {
            ensure!(
                !frame.data().hw_frames_ctx.is_null()
                    && frame.data().format == **format,
                "The hardware decoder returned an unexpected frame format"
            );
        }

        Ok(Receive::Item(frame))
    }
}

impl Drop for Codec {
    fn drop(&mut self) {
        let mut handle = self.raw();

        // SAFETY: Freeing the context joins codec workers before its callback storage is dropped
        unsafe { ffi::avcodec_free_context(&mut handle) };
    }
}
