//! Decoded audio waiting to play, which SDL pulls at the playback rate
//!
//! The demux worker pushes samples at media speed. When SDL needs output, the feed
//! either hands samples over for SDL to resample by the rate, so pitch follows speed,
//! or stretches them with Signalsmith Stretch, so pitch stays. Rate changes apply to
//! the next request, so nothing queued has to be dropped
//!
//! When a speed change moves from one way to the other, as lowering pitch only below
//! normal speed does at 1×, the old way plays on briefly and fades into the new one
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
/// How long the old way of producing audio fades into the new one
const FADE_SECONDS: f64 = 0.03;

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
    /// Whether audio is stretched at the current rate, rather than resampled by SDL
    stretching: bool,
    stretch: Stretch,
    /// Fractional input frames owed to the stretcher between requests
    carry: f64,
    /// The previous way of producing audio, still fading out
    fade: Option<Fade>,
    input: Vec<f32>,
    output: Vec<f32>,
    fading: Vec<f32>,
}

/// Audio from before a switch between stretching and resampling, which plays on at
/// media speed under the new audio until it has faded out
struct Fade {
    /// Next frame of the old audio
    read: usize,
    /// Whether the old audio came from the stretcher, which keeps running for it
    stretched: bool,
    /// Frames already faded
    done: usize,
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
                stretching: pitch.stretches(rate),
                stretch: Stretch::preset_default(CHANNELS as u32, RATE as u32),
                carry: 0.,
                fade: None,
                input: Vec::new(),
                output: Vec::new(),
                fading: Vec::new(),
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
        state.fade = None;
        state.restart_stretch();
    }

    /// Moves playback to `position` within the queued or recently played audio
    ///
    /// Positions past the end drop everything queued and wait for later samples;
    /// positions before the kept history fail, and the caller seeks instead
    ///
    /// Reading resumes far enough past `position` that, after the stretcher's delay and
    /// `latency`, the sample heard first is the one at `position`; the caller first
    /// drops what SDL still holds
    pub fn move_to(&self, position: f64, latency: f64) -> bool {
        self.lock().move_to(position, latency)
    }

    /// Changes speed from the next request on; `queued_bytes` and `latency` describe
    /// what SDL holds, so a switch between stretching and resampling keeps the heard
    /// position
    pub fn set_rate(&self, rate: f64, queued_bytes: usize, latency: f64) {
        let queued = queued_bytes as f64 / BYTES_PER_SECOND as f64;

        self.lock().set_rate(rate, queued, latency);
    }

    /// Changes how speed affects pitch; the caller then moves to the heard position,
    /// since what SDL holds was produced the old way
    pub fn set_pitch(&self, pitch: SpeedPitch) {
        let mut state = self.lock();
        state.pitch = pitch;

        let stretching = pitch.stretches(state.rate);
        if state.stretching != stretching {
            state.stretching = stretching;
            state.fade = None;
            state.restart_stretch();
        }
    }

    /// Whether audio is stretched at the current rate, so SDL must not resample it
    pub fn stretching(&self) -> bool {
        self.lock().stretching
    }

    /// Media time of the sample being heard, given what SDL still holds and the
    /// device latency
    pub fn heard_position(&self, queued_bytes: usize, latency: f64) -> Option<f64> {
        let queued = queued_bytes as f64 / BYTES_PER_SECOND as f64;

        self.lock().heard(queued, latency)
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

    fn heard(&self, queued: f64, latency: f64) -> Option<f64> {
        let next = self.start? + self.read as f64 / RATE as f64;

        Some(next - self.delay(queued, latency))
    }

    fn set_rate(&mut self, rate: f64, queued: f64, latency: f64) {
        let heard = self.heard(queued, latency);
        self.rate = rate;

        let stretching = self.pitch.stretches(rate);
        if stretching == self.stretching {
            return;
        }

        self.switch(stretching, heard, queued, latency);
    }

    /// Starts producing audio the other way from where it is heard, fading the old way
    /// out underneath
    fn switch(
        &mut self,
        stretching: bool,
        heard: Option<f64>,
        queued: f64,
        latency: f64,
    ) {
        let old_read = self.read;
        self.stretching = stretching;
        self.fade = None;

        let (Some(heard), Some(start)) = (heard, self.start) else {
            self.restart_stretch();
            return;
        };

        // The new way reads far enough ahead that its first audio heard continues on
        let read_at = heard + self.delay(queued, latency);
        let read = ((read_at - start) * RATE as f64).round().max(0.) as usize;
        self.read = read.min(self.frames());

        if stretching {
            self.restart_stretch();
            self.preroll();
        }

        self.fade = Some(Fade {
            read: old_read,
            stretched: !stretching,
            done: 0,
        });
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

    /// Media seconds between the next sample read and the sample being heard
    ///
    /// SDL holds unconverted samples at media speed when it resamples, and already
    /// stretched output otherwise, which the stretcher delays further
    fn delay(&self, queued: f64, latency: f64) -> f64 {
        let rate = self.rate;

        if !self.stretching {
            return queued + latency * rate;
        }

        let input = self.stretch.input_latency() as f64 / RATE as f64;
        let output = self.stretch.output_latency() as f64 / RATE as f64;
        input + (output + queued + latency) * rate
    }

    fn move_to(&mut self, position: f64, latency: f64) -> bool {
        let (Some(start), Some(end)) = (self.start, self.end()) else {
            return false;
        };

        if position < start - 0.001 {
            return false;
        }

        let read_at = position + self.delay(0., latency);

        if read_at >= end {
            self.samples.clear();
            self.start = Some(read_at);
            self.read = 0;
        } else {
            self.read = ((read_at - start) * RATE as f64).round() as usize;
        }

        self.fade = None;
        self.restart_stretch();
        self.preroll();
        true
    }

    /// Gives the stretcher the audio just before the read position, so its output
    /// starts there at full level instead of fading in
    fn preroll(&mut self) {
        if !self.stretching {
            return;
        }

        let frames = self.read.min(self.stretch.input_latency() * 2);
        if frames == 0 {
            return;
        }

        let first = (self.read - frames) * CHANNELS;

        self.input.clear();
        self.input
            .extend(self.samples.range(first..self.read * CHANNELS));
        self.stretch.seek(&self.input, self.rate.max(SILENT_RATE));
    }

    fn restart_stretch(&mut self) {
        self.stretch.reset();
        self.carry = 0.;
    }

    fn fill(&mut self, frames: usize) {
        self.output.clear();

        if self.start.is_none() || self.rate < SILENT_RATE {
            self.fade = None;
            self.output.resize(frames * CHANNELS, 0.);
            return;
        }

        if self.stretching {
            self.stretch_out(frames);
        } else {
            self.copy_out(frames);
        }

        if let Some(fade) = self.fade.take() {
            self.fade_out(fade, frames);
        }

        self.forget_history();
    }

    /// Mixes the old audio, fading out, into the output, whose new audio fades in
    fn fade_out(&mut self, mut fade: Fade, frames: usize) {
        let length = (FADE_SECONDS * RATE as f64) as usize;
        let count = frames.min(length - fade.done);
        let first = fade.read * CHANNELS;
        let available = self.frames().saturating_sub(fade.read).min(count);

        self.input.clear();
        self.input
            .extend(self.samples.range(first..first + available * CHANNELS));
        self.input.resize(count * CHANNELS, 0.);
        fade.read += count;

        // The old audio plays at media speed, which is close to the speed at a switch
        self.fading.clear();
        if fade.stretched {
            self.fading.resize(count * CHANNELS, 0.);
            self.stretch.process(&self.input, &mut self.fading);
        } else {
            self.fading.extend_from_slice(&self.input);
        }

        for index in 0..frames {
            let progress = ((fade.done + index) as f64 / length as f64).min(1.);
            let angle = progress * std::f64::consts::FRAC_PI_2;
            let (fade_in, fade_out) = (angle.sin() as f32, angle.cos() as f32);

            for channel in 0..CHANNELS {
                let sample = index * CHANNELS + channel;
                let old = self.fading.get(sample).copied().unwrap_or(0.);
                self.output[sample] = self.output[sample] * fade_in + old * fade_out;
            }
        }

        fade.done += count;
        if fade.done < length {
            self.fade = Some(fade);
        } else if fade.stretched {
            self.restart_stretch();
        }
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
    ///
    /// When less input is queued, as at the end of the file, the rest is stretched
    /// into proportionally fewer frames and followed by silence
    fn stretch_out(&mut self, frames: usize) {
        let wanted = frames as f64 * self.rate + self.carry;
        let available = self.unplayed_frames();
        let (needed, produced) = if available as f64 >= wanted.floor() {
            (wanted.floor() as usize, frames)
        } else {
            (
                available,
                ((available as f64 / self.rate) as usize).min(frames),
            )
        };

        self.carry = if produced == frames {
            wanted - needed as f64
        } else {
            0.
        };

        let first = self.read * CHANNELS;
        self.input.clear();
        self.input
            .extend(self.samples.range(first..first + needed * CHANNELS));
        self.read += needed;

        self.output.resize(produced * CHANNELS, 0.);
        if produced > 0 {
            self.stretch.process(&self.input, &mut self.output);
        }
        self.output.resize(frames * CHANNELS, 0.);
    }

    fn forget_history(&mut self) {
        let keep = (HISTORY_SECONDS * RATE as f64) as usize;
        let drop = self.read.saturating_sub(keep);

        if drop == 0 {
            return;
        }

        self.samples.drain(..drop * CHANNELS);
        self.read -= drop;
        if let Some(fade) = &mut self.fade {
            fade.read = fade.read.saturating_sub(drop);
        }
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
    fn moving_lands_the_heard_position_exactly_despite_the_stretcher_delay() {
        let feed = AudioFeed::new(0.25, SpeedPitch::Keep);
        feed.push(&tone(5.), 0.);
        pull(&feed, RATE * 2);

        // Repeated moves to where playback is heard must not creep backward
        for _ in 0..20 {
            let heard = feed.heard_position(0, 0.02).expect("A position");
            assert!(feed.move_to(heard, 0.02));
            let moved = feed.heard_position(0, 0.02).expect("A position");
            assert!((moved - heard).abs() < 1e-3, "{heard} became {moved}");
        }
    }

    #[test]
    fn keeping_pitch_plays_out_the_last_partial_block() {
        let feed = AudioFeed::new(1.5, SpeedPitch::Keep);
        feed.push(&tone(0.1), 0.);

        for _ in 0..10 {
            pull(&feed, 1024);
        }

        assert_eq!(feed.ahead(), 0., "The end of the audio was never played");
    }

    #[test]
    fn lowering_only_resamples_below_normal_speed_and_stretches_above() {
        let feed = AudioFeed::new(0.5, SpeedPitch::Lower);
        assert!(!feed.stretching());

        feed.set_rate(1.5, 0, 0.);
        assert!(feed.stretching());

        feed.set_rate(1., 0, 0.);
        assert!(!feed.stretching());
    }

    /// The largest jump between neighboring left samples
    fn largest_step(samples: &[f32]) -> f32 {
        let left: Vec<f32> = samples.iter().step_by(CHANNELS).copied().collect();

        left.windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0., f32::max)
    }

    #[test]
    fn crossing_normal_speed_keeps_the_heard_position_and_fades_between_ways() {
        for (from, to) in [(0.95, 1.05), (1.05, 0.95)] {
            let feed = AudioFeed::new(from, SpeedPitch::Lower);
            feed.push(&tone(4.), 0.);
            pull(&feed, RATE);

            let before = pull(&feed, 256);
            let heard = feed.heard_position(0, 0.02).expect("A position");
            feed.set_rate(to, 0, 0.02);
            let moved = feed.heard_position(0, 0.02).expect("A position");
            assert!((moved - heard).abs() < 1e-3, "{heard} became {moved}");

            let after = pull(&feed, RATE / 10);
            let joined: Vec<f32> = before.into_iter().chain(after).collect();

            // A 440 Hz tone moves at most about 0.06 between samples
            let step = largest_step(&joined);
            assert!(step < 0.12, "{from}× to {to}× jumped by {step}");
        }
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

        assert!(feed.move_to(2.5, 0.));
        assert!((feed.ahead() - 2.5).abs() < 0.01);
        assert!(
            !feed.move_to(0.5, 0.),
            "Audio older than the history was kept"
        );
        assert!(
            feed.move_to(8., 0.),
            "Moving past the end should wait for later audio"
        );
        assert_eq!(feed.end(), Some(8.));
    }
}
