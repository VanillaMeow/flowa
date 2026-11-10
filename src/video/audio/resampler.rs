//! Conversion from any source layout, format, and rate to interleaved stereo floats

use std::{ptr, ptr::NonNull};

use anyhow::{Context, Result, ensure};

use super::{CHANNELS, RATE};
use crate::video::{
    ffi::ffmpeg as ffi,
    ffmpeg::{AvFrame, check},
};

const MAX_RESAMPLED_FRAMES: i32 = 480_000;

/// Resampled samples and how far they lag behind the source frame
pub(super) struct Resampled {
    pub samples: Vec<f32>,
    /// Seconds of audio the resampler still holds back
    pub delay: f64,
}

pub(super) struct Resampler(NonNull<ffi::SwrContext>);

impl Resampler {
    /// Creates a resampler for the decoder's output format
    pub fn new(source: &ffi::AVCodecContext) -> Result<Self> {
        let mut stereo = ffi::AVChannelLayout::default();
        let mut resampler = ptr::null_mut();

        // SAFETY: FFmpeg initializes the temporary layout and copies it into the new resampler
        let result = unsafe {
            ffi::av_channel_layout_default(&mut stereo, CHANNELS as i32);

            let result = ffi::swr_alloc_set_opts2(
                &mut resampler,
                &stereo,
                ffi::AV_SAMPLE_FMT_FLT,
                RATE as i32,
                &source.ch_layout,
                source.sample_fmt,
                source.sample_rate,
                0,
                ptr::null_mut(),
            );

            ffi::av_channel_layout_uninit(&mut stereo);
            result
        };

        if result < 0 {
            unsafe { ffi::swr_free(&mut resampler) };
            check(result, "Cannot allocate the audio resampler")?;
        }

        let resampler = Self(
            NonNull::new(resampler).context("Cannot allocate the audio resampler")?,
        );

        check(
            unsafe { ffi::swr_init(resampler.0.as_ptr()) },
            "Cannot initialize the audio resampler",
        )?;

        Ok(resampler)
    }

    /// Discards buffered samples after a seek
    pub fn reset(&mut self) -> Result<()> {
        // SAFETY: Only the audio worker owns this resampler
        unsafe { ffi::swr_close(self.0.as_ptr()) };
        check(
            unsafe { ffi::swr_init(self.0.as_ptr()) },
            "Cannot reset the audio resampler",
        )?;

        Ok(())
    }

    pub fn convert(&mut self, frame: &AvFrame) -> Result<Resampled> {
        let data = frame.data();
        let rate = RATE as i32;

        // SAFETY: The resampler is initialized and the frame belongs to this worker
        let delay = unsafe { ffi::swr_get_delay(self.0.as_ptr(), i64::from(rate)) }
            as f64
            / f64::from(rate);
        let capacity =
            unsafe { ffi::swr_get_out_samples(self.0.as_ptr(), data.nb_samples) };
        ensure!(
            (1..=MAX_RESAMPLED_FRAMES).contains(&capacity),
            "Invalid resampled audio capacity"
        );

        let mut samples = vec![0.0_f32; capacity as usize * CHANNELS];
        let mut planes = [samples.as_mut_ptr().cast::<u8>()];

        // SAFETY: Output fits the requested stereo capacity and FFmpeg owns the input planes
        let frames = check(
            unsafe {
                ffi::swr_convert(
                    self.0.as_ptr(),
                    planes.as_mut_ptr(),
                    capacity,
                    data.extended_data.cast(),
                    data.nb_samples,
                )
            },
            "Cannot resample audio",
        )?;
        ensure!(frames <= capacity, "Invalid resampled audio size");
        samples.truncate(frames as usize * CHANNELS);

        Ok(Resampled { samples, delay })
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        let mut resampler = self.0.as_ptr();

        // SAFETY: This wrapper owns the resampler and no conversion remains active
        unsafe { ffi::swr_free(&mut resampler) };
    }
}
