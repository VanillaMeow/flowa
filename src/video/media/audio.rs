//! Owned audio codec and stereo resampling with Rust-owned sample buffers

use std::{ptr, ptr::NonNull};

use anyhow::{Context, Result, ensure};

use super::{Codec, Packet, Receive, Sent, StreamInfo, check, ffi, timestamp};

const RATE: i32 = 48_000;
const CHANNELS: usize = 2;
const MAX_FRAMES: i32 = 480_000;

pub(super) struct AudioDecoder {
    codec: Codec,
    resampler: NonNull<ffi::SwrContext>,
}

impl AudioDecoder {
    pub fn open(stream: &ffi::AVStream) -> Result<Self> {
        let codec = Codec::software_stream(stream)?;
        let mut stereo = ffi::AVChannelLayout::default();
        let mut resampler = ptr::null_mut();

        // SAFETY: FFmpeg initializes the temporary layout and copies it into the new resampler
        let result = unsafe {
            ffi::av_channel_layout_default(&mut stereo, CHANNELS as i32);

            let context = &*codec.raw();
            let result = ffi::swr_alloc_set_opts2(
                &mut resampler,
                &stereo,
                ffi::AV_SAMPLE_FMT_FLT,
                RATE,
                &context.ch_layout,
                context.sample_fmt,
                context.sample_rate,
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

        let resampler =
            NonNull::new(resampler).context("Cannot allocate the audio resampler")?;
        let audio = Self { codec, resampler };

        check(
            unsafe { ffi::swr_init(audio.resampler.as_ptr()) },
            "Cannot initialize the audio resampler",
        )?;

        Ok(audio)
    }

    pub fn flush(&mut self) -> Result<()> {
        self.codec.flush();

        // SAFETY: Only the audio worker owns this resampler
        unsafe { ffi::swr_close(self.resampler.as_ptr()) };
        check(
            unsafe { ffi::swr_init(self.resampler.as_ptr()) },
            "Cannot reset the audio resampler",
        )?;

        Ok(())
    }

    pub fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        self.codec.send(packet)
    }

    pub fn receive(&mut self) -> Result<Receive<AudioBlock>> {
        let frame = match self.codec.receive()? {
            Receive::Again => return Ok(Receive::Again),
            Receive::End => return Ok(Receive::End),
            Receive::Item(frame) => frame,
        };
        let data = frame.data();

        // SAFETY: The resampler is initialized and the frame belongs to this worker
        let delay =
            unsafe { ffi::swr_get_delay(self.resampler.as_ptr(), i64::from(RATE)) }
                as f64
                / f64::from(RATE);
        let capacity = unsafe {
            ffi::swr_get_out_samples(self.resampler.as_ptr(), data.nb_samples)
        };
        ensure!(
            (1..=MAX_FRAMES).contains(&capacity),
            "Invalid resampled audio capacity"
        );

        let mut samples = vec![0.0_f32; capacity as usize * CHANNELS];
        let mut planes = [samples.as_mut_ptr().cast::<u8>()];

        // SAFETY: Output fits the requested stereo capacity and FFmpeg owns the input planes
        let frames = check(
            unsafe {
                ffi::swr_convert(
                    self.resampler.as_ptr(),
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

        Ok(Receive::Item(AudioBlock {
            samples,
            timestamp: data.best_effort_timestamp,
            resample_delay: delay,
        }))
    }
}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        let mut resampler = self.resampler.as_ptr();

        // SAFETY: This wrapper owns the resampler and no conversion remains active
        unsafe { ffi::swr_free(&mut resampler) };
    }
}

pub(in crate::video) struct AudioBlock {
    samples: Vec<f32>,
    timestamp: i64,
    resample_delay: f64,
}

impl AudioBlock {
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn position(&self, info: &StreamInfo, previous: f64) -> f64 {
        timestamp(self.timestamp, info.audio_time_base, info.origin).unwrap_or(previous)
            - self.resample_delay
    }
}
