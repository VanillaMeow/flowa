//! Finds the FFmpeg decoder and pixel format for a hardware device, and holds FFmpeg
//! to that format

use std::{ffi::c_void, ptr};

use anyhow::{Result, bail};

use crate::video::{
    ffi::ffmpeg as ffi,
    ffmpeg::{CodecSetup, HwDevice},
};

/// A decoder that outputs frames of one hardware pixel format on one device
pub(super) struct HardwareFormat {
    decoder: *const ffi::AVCodec,
    // FFmpeg reads this through the context's opaque pointer during get_format
    format: ffi::AVPixelFormat,
    device: Option<HwDevice>,
}

impl HardwareFormat {
    /// Finds a decoder for the stream's codec that can decode on the device
    pub fn negotiate(stream: &ffi::AVStream, device: HwDevice) -> Result<Self> {
        let device_type = device.kind();
        let mut iterator = ptr::null_mut();

        // SAFETY: FFmpeg returns static descriptions while the source owns its codec parameters
        unsafe {
            loop {
                let decoder = ffi::av_codec_iterate(&mut iterator);
                if decoder.is_null() {
                    bail!("The hardware backend does not support this codec");
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
                        return Ok(Self {
                            decoder,
                            format: (*configuration).pix_fmt,
                            device: Some(device),
                        });
                    }

                    index += 1;
                }
            }
        }
    }

    pub fn format(&self) -> ffi::AVPixelFormat {
        self.format
    }
}

impl CodecSetup for HardwareFormat {
    fn decoder(&self, _stream: &ffi::AVStream) -> *const ffi::AVCodec {
        self.decoder
    }

    fn configure(&mut self, context: &mut ffi::AVCodecContext) {
        if let Some(device) = self.device.take() {
            context.hw_device_ctx = device.into_raw();
        }

        // The codec context keeps this setup boxed, so the format's address is stable
        context.get_format = Some(hardware_format);
        context.opaque = ptr::from_mut(&mut self.format).cast::<c_void>();
        context.thread_count = 1;
        context.thread_type = 0;
    }
}

unsafe extern "C" fn hardware_format(
    context: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    // SAFETY: The codec context retains the boxed setup until avcodec_free_context completes
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
