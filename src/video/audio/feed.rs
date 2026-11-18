//! Decoded audio waiting to play, which SDL pulls at the playback rate
//!
//! The demux worker pushes samples at media speed. When SDL needs output, the feed
//! either hands samples over for SDL to resample by the rate, so pitch follows speed,
//! or stretches them with Signalsmith Stretch, so pitch stays. Rate changes apply to
//! the next request, so nothing queued has to be dropped
//!
//! Played samples stay for a few seconds, so playback can resume a little earlier
//! after stepping back without reading the file again

use std::{collections::VecDeque, sync::Mutex};

use signalsmith_stretch::Stretch;

use super::{BYTES_PER_SECOND, CHANNELS, RATE};
use crate::settings::SpeedPitch;

/// Played audio kept for resuming after backward steps
const HISTORY_SECONDS: f64 = 3.;
/// Gaps in the audio up to this long become silence; longer ones restart the feed
const MAX_GAP_SECONDS: f64 = 30.;
/// Below this rate the output is silent and nothing is consumed
pub(in crate::video) const SILENT_RATE: f64 = 0.01;

pub(in crate::video) struct AudioFeed {
    state: Mutex<FeedState>,
}

struct FeedState {
    /// Interleaved stereo at the output rate, at media speed
    samples: VecDeque<f32>,
    /// Media time of the first sample, once any arrived since the last clear
    start: Option<f64>,
    /// Frames already played
    read: usize,
    rate: f64,
    pitch: SpeedPitch,
    stretch: Stretch,
    /// Fractional input frames owed to the stretcher between requests
    carry: f64,
    input: Vec<f32>,
    output: Vec<f32>,
}

impl AudioFeed {
    pub fn new(rate: f64, pitch: SpeedPitch) -> Self {
        Self {
            state: Mutex::new(FeedState {
                samples: VecDeque::new(),
                start: None,
                read: 0,
                rate,
                pitch,
                stretch: Stretch::preset_default(CHANNELS as u32, RATE as u32),
                carry: 0.,
                input: Vec::new(),
                output: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FeedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Adds samples that start at `position`, filling short gaps with silence and
    /// trimming overlap with what is already queued
    pub fn push(&self, samples: &[f32], position: f64) {
        self.lock().push(samples, position);
    }

    /// Media time after the last queued sample
    pub fn end(&self) -> Option<f64> {
        self.lock().end()
    }

    /// Seconds of media queued beyond the playing position
    pub fn ahead(&self) -> f64 {
        let state = self.lock();

        state.unplayed_frames() as f64 / RATE as f64
    }

    /// Forgets everything, as after a seek
    pub fn clear(&self) {
        let mut state = self.lock();
        state.samples.clear();
        state.start = None;
        state.read = 0;
        state.restart_stretch();
    }

    /// Moves playback to `position` within the queued or recently played audio
    ///
    /// Positions past the end drop everything queued and wait for later samples;
    /// positions before the kept history fail, and the caller seeks instead
    pub fn move_to(&self, position: f64) -> bool {
        self.lock().move_to(position)
    }

    pub fn set_rate(&self, rate: f64) {
        self.lock().rate = rate;
    }

    pub fn set_pitch(&self, pitch: SpeedPitch) {
        let mut state = self.lock();

        if state.pitch != pitch {
            state.pitch = pitch;
            state.restart_stretch();
        }
    }

    /// Media time of the sample being heard, given what SDL still holds and the
    /// device latency
    pub fn heard_position(&self, queued_bytes: usize, latency: f64) -> Option<f64> {
        let state = self.lock();
        let next = state.start? + state.read as f64 / RATE as f64;
        let queued = queued_bytes as f64 / BYTES_PER_SECOND as f64;
        let rate = state.rate;

        // SDL holds unconverted samples at media speed when it resamples, and
        // already stretched output otherwise, which the stretcher delays further
        let delay = match state.pitch {
            SpeedPitch::Follow => queued + latency * rate,
            SpeedPitch::Keep => {
                let input = state.stretch.input_latency() as f64 / RATE as f64;
                let output = state.stretch.output_latency() as f64 / RATE as f64;
                input + (output + queued + latency) * rate
            }
        };

        Some(next - delay)
    }

    /// Produces up to `frames` frames of output for SDL and passes them to `put`
    pub fn fill(&self, frames: usize, put: impl FnOnce(&[f32])) {
        let mut state = self.lock();
        let state = &mut *state;

        state.fill(frames);
        put(&state.output);
    }
}

impl FeedState {
    fn frames(&self) -> usize {
        self.samples.len() / CHANNELS
    }

    fn unplayed_frames(&self) -> usize {
        self.frames().saturating_sub(self.read)
    }

    fn end(&self) -> Option<f64> {
        Some(self.start? + self.frames() as f64 / RATE as f64)
    }

    fn push(&mut self, samples: &[f32], position: f64) {
        let Some(end) = self.end() else {
            self.start = Some(position);
            self.samples.extend(samples);
            return;
        };

        let gap = position - end;

        if gap > MAX_GAP_SECONDS {
            self.samples.clear();
            self.start = Some(position);
            self.read = 0;
            self.samples.extend(samples);
            return;
        }

        let mut samples = samples;

        if gap > 0.01 {
            let silence = (gap * RATE as f64).round() as usize * CHANNELS;
            self.samples.extend(std::iter::repeat_n(0., silence));
        } else if gap < -0.01 {
            let overlap =
                ((-gap * RATE as f64).round() as usize * CHANNELS).min(samples.len());
            samples = &samples[overlap..];
        }

        self.samples.extend(samples);
    }

    fn move_to(&mut self, position: f64) -> bool {
        let (Some(start), Some(end)) = (self.start, self.end()) else {
            return false;
        };

        if position < start - 0.001 {
            return false;
        }

        if position >= end {
            self.samples.clear();
            self.start = Some(position);
            self.read = 0;
        } else {
            self.read = ((position - start) * RATE as f64).round() as usize;
        }

        self.restart_stretch();
        true
    }

    fn restart_stretch(&mut self) {
        self.stretch.reset();
        self.carry = 0.;
    }

    fn fill(&mut self, frames: usize) {
        self.output.clear();

        if self.start.is_none() || self.rate < SILENT_RATE {
            self.output.resize(frames * CHANNELS, 0.);
            return;
        }

        match self.pitch {
            SpeedPitch::Follow => self.copy_out(frames),
            SpeedPitch::Keep => self.stretch_out(frames),
        }

        self.forget_history();
    }

    /// Hands over media-speed samples; SDL resamples them by the rate
    fn copy_out(&mut self, frames: usize) {
        let available = self.unplayed_frames().min(frames);
        let first = self.read * CHANNELS;

        self.output
            .extend(self.samples.range(first..first + available * CHANNELS));
        self.read += available;

        // Underruns play silence without consuming anything
        self.output.resize(frames * CHANNELS, 0.);
    }

    /// Stretches `frames × rate` input frames into `frames` output frames
    fn stretch_out(&mut self, frames: usize) {
        let wanted = frames as f64 * self.rate + self.carry;
        let needed = wanted.floor() as usize;

        if self.unplayed_frames() < needed {
            self.output.resize(frames * CHANNELS, 0.);
            return;
        }

        self.carry = wanted - needed as f64;

        let first = self.read * CHANNELS;
        self.input.clear();
        self.input
            .extend(self.samples.range(first..first + needed * CHANNELS));
        self.read += needed;

        self.output.resize(frames * CHANNELS, 0.);
        self.stretch.process(&self.input, &mut self.output);
    }

    fn forget_history(&mut self) {
        let keep = (HISTORY_SECONDS * RATE as f64) as usize;
        let drop = self.read.saturating_sub(keep);

        if drop == 0 {
            return;
        }

        self.samples.drain(..drop * CHANNELS);
        self.read -= drop;
        self.start = self.start.map(|start| start + drop as f64 / RATE as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f64) -> Vec<f32> {
        (0..(seconds * RATE as f64) as usize)
            .flat_map(|index| {
                let time = index as f64 / RATE as f64;
                [(time * 440. * std::f64::consts::TAU).sin() as f32; CHANNELS]
            })
            .collect()
    }

    fn pull(feed: &AudioFeed, frames: usize) -> Vec<f32> {
        let mut pulled = Vec::new();
        feed.fill(frames, |samples| pulled.extend_from_slice(samples));
        pulled
    }

    #[test]
    fn gaps_become_silence_and_overlap_is_trimmed() {
        let feed = AudioFeed::new(1., SpeedPitch::Follow);
        feed.push(&vec![1.; RATE * CHANNELS], 0.);
        feed.push(&vec![1.; RATE * CHANNELS / 2], 1.5);
        assert_eq!(feed.end(), Some(2.));

        feed.push(&vec![1.; RATE * CHANNELS], 1.5);
        assert_eq!(feed.end(), Some(2.5));

        let first = pull(&feed, RATE * 3 / 2);
        assert!(first[..RATE * CHANNELS].iter().all(|&sample| sample == 1.));
        assert!(first[RATE * CHANNELS..].iter().all(|&sample| sample == 0.));
    }

    #[test]
    fn following_pitch_consumes_what_sdl_asks_for_and_tracks_the_position() {
        let feed = AudioFeed::new(2., SpeedPitch::Follow);
        feed.push(&tone(2.), 10.);

        pull(&feed, RATE / 2);
        let heard = feed.heard_position(0, 0.).expect("A position");
        assert!((heard - 10.5).abs() < 1e-9);

        // Queued media-speed samples and latency have not been heard yet
        let queued = RATE / 10 * CHANNELS * size_of::<f32>();
        let heard = feed.heard_position(queued, 0.05).expect("A position");
        assert!((heard - (10.5 - 0.1 - 0.1)).abs() < 1e-9);
    }

    #[test]
    fn keeping_pitch_consumes_input_at_the_rate() {
        let feed = AudioFeed::new(0.5, SpeedPitch::Keep);
        feed.push(&tone(4.), 0.);

        for _ in 0..10 {
            pull(&feed, RATE / 10);
        }

        // One second of output at half speed plays half a second of media
        assert!((feed.ahead() - 3.5).abs() < 0.01, "{} left", feed.ahead());
    }

    #[test]
    fn keeping_pitch_at_half_speed_still_plays_440_hz() {
        let feed = AudioFeed::new(0.5, SpeedPitch::Keep);
        feed.push(&tone(3.), 0.);

        // Skip the stretcher's start-up, then count zero crossings over a second
        pull(&feed, RATE / 2);
        let output = pull(&feed, RATE);
        let left: Vec<f32> = output.iter().step_by(CHANNELS).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|pair| (pair[0] < 0.) != (pair[1] < 0.))
            .count();

        let frequency = crossings as f64 / 2.;
        assert!((frequency - 440.).abs() < 10., "Heard {frequency} Hz");
    }

    #[test]
    fn zero_rate_is_silent_and_holds_the_position() {
        let feed = AudioFeed::new(0., SpeedPitch::Keep);
        feed.push(&tone(1.), 0.);

        let output = pull(&feed, 1024);
        assert!(output.iter().all(|&sample| sample == 0.));
        assert_eq!(feed.ahead(), 1.);
    }

    #[test]
    fn moving_within_the_kept_history_rewinds_without_reading_again() {
        let feed = AudioFeed::new(1., SpeedPitch::Follow);
        feed.push(&tone(5.), 0.);
        pull(&feed, RATE * 4);

        assert!(feed.move_to(2.5));
        assert!((feed.ahead() - 2.5).abs() < 0.01);
        assert!(!feed.move_to(0.5), "Audio older than the history was kept");
        assert!(
            feed.move_to(8.),
            "Moving past the end should wait for later audio"
        );
        assert_eq!(feed.end(), Some(8.));
    }
}
