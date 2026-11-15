//! Changes the speed of output-format audio without changing its pitch
//!
//! FFmpeg's `atempo` filter overlaps and adds windows of the input, so voices keep
//! their pitch at any playback rate. It accepts factors from 0.5 to 100, so slower
//! rates chain several filters

use std::{ffi::CString, ptr, ptr::NonNull};

use anyhow::{Context, Result, ensure};

use super::{CHANNELS, RATE};
use crate::video::{
    ffi::ffmpeg as ffi,
    ffmpeg::{AvFrame, check},
};

/// The slowest factor one `atempo` filter accepts
const SLOWEST_FILTER: f64 = 0.5;

pub(in crate::video) struct Tempo {
    graph: NonNull<ffi::AVFilterGraph>,
    source: NonNull<ffi::AVFilterContext>,
    sink: NonNull<ffi::AVFilterContext>,
    output: AvFrame,
    /// Input frames sent so far, which time-stamp the next input
    sent: i64,
}

// SAFETY: The graph and its frames are owned by one worker at a time and never shared
unsafe impl Send for Tempo {}

impl Tempo {
    pub fn new(rate: f64) -> Result<Self> {
        ensure!(rate.is_finite() && rate > 0., "Invalid playback rate");

        // SAFETY: The new graph is owned by this wrapper, which frees it on every path
        let graph = NonNull::new(unsafe { ffi::avfilter_graph_alloc() })
            .context("Cannot allocate the audio tempo filter")?;
        let mut tempo = Self {
            graph,
            source: NonNull::dangling(),
            sink: NonNull::dangling(),
            output: AvFrame::allocate()?,
            sent: 0,
        };

        tempo.build(rate)?;
        Ok(tempo)
    }

    /// Stretches interleaved stereo samples, appending whatever the filter releases
    pub fn push(&mut self, samples: &[f32], output: &mut Vec<f32>) -> Result<()> {
        let frames = samples.len() / CHANNELS;

        if frames == 0 {
            return Ok(());
        }

        let input = input_frame(samples, self.sent)?;
        self.sent += frames as i64;

        // SAFETY: The source takes the frame's buffer references and resets the frame
        check(
            unsafe {
                ffi::av_buffersrc_add_frame_flags(self.source.as_ptr(), input.raw(), 0)
            },
            "Cannot stretch audio",
        )?;

        self.drain(output)
    }

    /// Releases the samples the filter still holds once the input has ended
    pub fn finish(&mut self, output: &mut Vec<f32>) -> Result<()> {
        // SAFETY: A null frame marks the end of the input
        check(
            unsafe {
                ffi::av_buffersrc_add_frame_flags(
                    self.source.as_ptr(),
                    ptr::null_mut(),
                    0,
                )
            },
            "Cannot finish stretching audio",
        )?;

        self.drain(output)
    }

    fn drain(&mut self, output: &mut Vec<f32>) -> Result<()> {
        loop {
            // SAFETY: The sink writes a new reference into the reused, unreferenced frame
            let result = unsafe {
                ffi::av_buffersink_get_frame(self.sink.as_ptr(), self.output.raw())
            };

            if result == ffi::FLOWA_AVERROR_AGAIN || result == ffi::FLOWA_AVERROR_EOF {
                return Ok(());
            }
            check(result, "Cannot read stretched audio")?;

            let frame = self.output.data();
            let count = usize::try_from(frame.nb_samples)? * CHANNELS;

            // SAFETY: Aformat delivers packed stereo floats in the first plane
            let samples = unsafe {
                std::slice::from_raw_parts(frame.data[0].cast::<f32>(), count)
            };
            output.extend_from_slice(samples);

            // SAFETY: The copy is done, so the frame's reference can be released
            unsafe { ffi::av_frame_unref(self.output.raw()) };
        }
    }

    fn build(&mut self, rate: f64) -> Result<()> {
        let input = format!(
            "sample_rate={RATE}:sample_fmt=flt:channel_layout=stereo:time_base=1/{RATE}"
        );
        let format =
            format!("sample_fmts=flt:sample_rates={RATE}:channel_layouts=stereo");

        self.source = self.add("abuffer", "input", Some(&input))?;
        let mut previous = self.source;

        for (index, factor) in filter_factors(rate).into_iter().enumerate() {
            let tempo = format!("tempo={factor}");
            let filter = self.add("atempo", &format!("tempo{index}"), Some(&tempo))?;
            link(previous, filter)?;
            previous = filter;
        }

        let packed = self.add("aformat", "format", Some(&format))?;
        link(previous, packed)?;
        self.sink = self.add("abuffersink", "output", None)?;
        link(packed, self.sink)?;

        // SAFETY: Every filter belongs to this graph and is linked
        check(
            unsafe { ffi::avfilter_graph_config(self.graph.as_ptr(), ptr::null_mut()) },
            "Cannot configure the audio tempo filter",
        )?;

        Ok(())
    }

    fn add(
        &mut self,
        filter: &str,
        name: &str,
        arguments: Option<&str>,
    ) -> Result<NonNull<ffi::AVFilterContext>> {
        let filter_name = CString::new(filter)?;
        let instance = CString::new(name)?;
        let arguments = arguments.map(CString::new).transpose()?;

        // SAFETY: FFmpeg returns a static filter definition or null
        let definition = unsafe { ffi::avfilter_get_by_name(filter_name.as_ptr()) };
        ensure!(!definition.is_null(), "FFmpeg lacks the {filter} filter");

        let mut context = ptr::null_mut();

        // SAFETY: The graph owns the new filter, and FFmpeg copies the strings
        check(
            unsafe {
                ffi::avfilter_graph_create_filter(
                    &mut context,
                    definition,
                    instance.as_ptr(),
                    arguments.as_ref().map_or(ptr::null(), |text| text.as_ptr()),
                    ptr::null_mut(),
                    self.graph.as_ptr(),
                )
            },
            "Cannot create the audio tempo filter",
        )?;

        NonNull::new(context).context("Cannot create the audio tempo filter")
    }
}

impl Drop for Tempo {
    fn drop(&mut self) {
        let mut graph = self.graph.as_ptr();

        // SAFETY: Freeing the graph frees its filters, which nothing uses afterward
        unsafe { ffi::avfilter_graph_free(&mut graph) };
    }
}

/// Splits a rate into factors that each `atempo` filter accepts
fn filter_factors(rate: f64) -> Vec<f64> {
    let filters = if rate < SLOWEST_FILTER {
        (rate.ln() / SLOWEST_FILTER.ln()).ceil() as i32
    } else {
        1
    };

    vec![rate.powf(1. / f64::from(filters)); filters as usize]
}

fn link(
    from: NonNull<ffi::AVFilterContext>,
    to: NonNull<ffi::AVFilterContext>,
) -> Result<()> {
    // SAFETY: Both filters belong to the same graph
    check(
        unsafe { ffi::avfilter_link(from.as_ptr(), 0, to.as_ptr(), 0) },
        "Cannot link the audio tempo filter",
    )?;

    Ok(())
}

/// Copies samples into a frame the filter's source accepts
fn input_frame(samples: &[f32], timestamp: i64) -> Result<AvFrame> {
    let frame = AvFrame::allocate()?;
    let frames = i32::try_from(samples.len() / CHANNELS)?;

    // SAFETY: The frame is newly allocated, and FFmpeg sizes its buffer from these fields
    unsafe {
        let raw = frame.raw();
        (*raw).nb_samples = frames;
        (*raw).format = ffi::AV_SAMPLE_FMT_FLT;
        (*raw).sample_rate = RATE as i32;
        (*raw).pts = timestamp;
        ffi::av_channel_layout_default(&mut (*raw).ch_layout, CHANNELS as i32);
        check(
            ffi::av_frame_get_buffer(raw, 0),
            "Cannot allocate stretched audio",
        )?;

        let destination = (*raw).data[0].cast::<f32>();
        ptr::copy_nonoverlapping(samples.as_ptr(), destination, samples.len());
    }

    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_rates_chain_filters_within_the_accepted_range() {
        assert_eq!(filter_factors(1.5), vec![1.5]);
        assert_eq!(filter_factors(0.5), vec![0.5]);

        let factors = filter_factors(0.25);
        assert_eq!(factors.len(), 2);
        assert!(factors.iter().all(|factor| (factor - 0.5).abs() < 1e-9));
    }

    #[test]
    fn stretching_keeps_pitch_and_scales_duration() -> Result<()> {
        let seconds = 2.;
        let frequency = 440.;
        let input: Vec<f32> = (0..(RATE as f64 * seconds) as usize)
            .flat_map(|index| {
                let time = index as f64 / RATE as f64;
                let sample = (time * frequency * std::f64::consts::TAU).sin() as f32;
                [sample; CHANNELS]
            })
            .collect();

        for rate in [2., 0.5] {
            let mut tempo = Tempo::new(rate)?;
            let mut output = Vec::new();

            for chunk in input.chunks(4096 * CHANNELS) {
                tempo.push(chunk, &mut output)?;
            }
            tempo.finish(&mut output)?;

            let duration = output.len() as f64 / CHANNELS as f64 / RATE as f64;
            ensure!(
                (duration - seconds / rate).abs() < 0.05,
                "Rate {rate} produced {duration} seconds"
            );

            // Zero crossings per second measure the pitch, which must stay at 440 Hz
            let left: Vec<f32> = output.iter().step_by(CHANNELS).copied().collect();
            let crossings = left
                .windows(2)
                .filter(|pair| (pair[0] < 0.) != (pair[1] < 0.))
                .count();
            let measured = crossings as f64 / 2. / duration;
            ensure!(
                (measured - frequency).abs() < 10.,
                "Rate {rate} changed the pitch to {measured} Hz"
            );
        }

        Ok(())
    }
}
