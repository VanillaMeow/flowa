//! Playback lifecycle, controls, frame scheduling, and ownership of the media worker

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    thread::JoinHandle,
    time::Instant,
};

use anyhow::{Context, Result, ensure};

use super::{
    frame_history::TimedFrame,
    state::{PlaybackState, Seek, Shared},
    stepping::{MAX_WAITING_STEPS, Stepping},
    worker,
};
use crate::settings::{SpeedPitch, VideoDecoder};
use crate::video::{
    MAX_RATE, VideoState,
    audio::{AudioOutput, SILENT_RATE},
    decoding::{DecodedFrame, DecoderSelection},
    ffmpeg::{self, Stream, StreamInfo},
    gpu::NativeGpu,
    subtitles::ActiveSubtitles,
};

/// Bounds how many late frames one poll may skip while catching up to the clock
const MAX_FRAMES_PER_POLL: usize = 12;

/// The UI-facing playback state plus scheduling details used by the player
#[derive(Clone)]
pub(super) struct Snapshot {
    pub video: VideoState,
    /// Seconds until the next decoded frame is due
    pub next_frame_delay: f64,
    /// Increments whenever a new frame becomes current
    pub frame_serial: u64,
    /// The SDL audio driver, such as `sdl3/pipewire`
    pub audio_output: String,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            video: VideoState::default(),
            next_frame_delay: 0.005,
            frame_serial: 0,
            audio_output: String::new(),
        }
    }
}

struct Resume {
    position: f64,
    paused: bool,
}

pub(super) struct Engine {
    gpu: Arc<NativeGpu>,
    ctx: egui::Context,
    shared: Option<Arc<Shared>>,
    worker: Option<JoinHandle<()>>,
    decoder: Option<DecoderSelection>,
    decoder_preference: VideoDecoder,
    path: Option<PathBuf>,
    resume: Option<Resume>,
    current: Option<TimedFrame>,
    /// The next frame, waiting for its time
    pending: Option<TimedFrame>,
    /// Kept frames and the background decoder, for stepping without waiting
    stepping: Stepping,
    applied_generation: u64,
    frame_serial: u64,
    last_pts: f64,
    volume: f64,
    rate: f64,
    pitch: SpeedPitch,
    /// Whether newly opened videos pick subtitles, cleared when the user turns them off
    subtitles_wanted: bool,
    /// Frames still to show while paused, one per forward step
    steps_forward: u32,
}

/// What background work did
#[derive(Default)]
pub(super) struct Background {
    /// More work remains
    pub busy: bool,
    /// Frames changed, so a poll should show them
    pub changed: bool,
}

impl Engine {
    pub fn new(gpu: Arc<NativeGpu>, ctx: egui::Context) -> Self {
        Self {
            gpu,
            ctx,
            shared: None,
            worker: None,
            decoder: None,
            decoder_preference: VideoDecoder::Auto,
            path: None,
            resume: None,
            current: None,
            pending: None,
            stepping: Stepping::default(),
            applied_generation: 0,
            frame_serial: 0,
            last_pts: 0.0,
            volume: 100.0,
            rate: 1.0,
            pitch: SpeedPitch::default(),
            subtitles_wanted: true,
            steps_forward: 0,
        }
    }

    pub fn open(&mut self, path: &Path) -> Result<()> {
        self.stop();
        let shared = Arc::new(Shared::new(
            self.ctx.clone(),
            self.volume,
            self.rate,
            self.pitch,
        ));
        shared.lock().subtitles_wanted = self.subtitles_wanted;
        let worker = worker::spawn(path.to_owned(), Arc::clone(&shared))?;
        self.shared = Some(shared);
        self.worker = Some(worker);
        self.path = Some(path.to_owned());
        Ok(())
    }

    pub fn stop(&mut self) {
        self.path = None;
        self.resume = None;
        let Some(shared) = self.shared.take() else {
            return;
        };

        shared.stop();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            log::warn!("The media worker terminated unexpectedly");
        }

        // The demux worker has joined before its frames and audio output are released
        self.pending = None;
        self.current = None;
        self.stepping.reset();
        self.decoder = None;
        self.frame_serial = 0;
        self.applied_generation = 0;
        self.last_pts = 0.0;
        self.steps_forward = 0;
        drop(shared);
    }

    pub fn toggle_pause(&mut self) -> Result<()> {
        let Some(shared) = &self.shared else {
            return Ok(());
        };
        let paused = shared.lock().paused;

        self.set_paused(!paused)
    }

    fn set_paused(&mut self, paused: bool) -> Result<()> {
        let Some(shared) = &self.shared else {
            return Ok(());
        };

        // The clock holds or continues from where playback is now
        let mut state = shared.lock();
        let now = Instant::now();
        let position = state.position(now)?;
        state.clock.anchor(position, now);
        state.paused = paused;
        let restart = state.ended && !paused;

        // After steps the audio is realigned to the picture, and resuming before the
        // audio still kept, as after stepping far back, reads it again
        let audible = state.audio.is_some() && state.audio_info.is_some();
        let (heard, _) = state.heard()?;
        let in_step = heard.is_some_and(|heard| (heard - position).abs() < 0.02);
        let realign =
            !paused && audible && !in_step && !state.realign_audio(position)?;
        if let Some(audio) = &state.audio {
            audio.pause(paused || !state.started)?;
        }
        drop(state);
        shared.notify();
        self.steps_forward = 0;

        // Playing reads on from the main decoder, which must not skip dropped frames
        let gap = !paused && self.stepping.resume();
        if restart {
            self.seek(0.0)?;
        } else if realign || gap {
            self.seek(position)?;
        }

        Ok(())
    }

    /// Pauses and shows the next frame, or the previous one
    pub fn step(&mut self, frames: i32) -> Result<()> {
        let Some(shared) = self.shared.as_ref().map(Arc::clone) else {
            return Ok(());
        };

        let Some(info) = stream_info(&shared.lock()) else {
            return Ok(());
        };

        self.set_paused(true)?;
        self.stepping.begin(frames > 0);
        if frames < 0 {
            return self.step_back(&shared, info, frames.unsigned_abs());
        }

        if shared.lock().ended {
            return Ok(());
        }

        // Forward steps show frames as they are ready, kept or decoded
        self.steps_forward =
            (self.steps_forward + frames as u32).min(MAX_WAITING_STEPS);
        self.plan_stepping(&shared, info);
        shared.notify();

        Ok(())
    }

    /// Shows kept frames from before the current one; steps beyond them wait for the
    /// background decoder, or seek when it cannot run
    fn step_back(
        &mut self,
        shared: &Shared,
        info: StreamInfo,
        frames: u32,
    ) -> Result<()> {
        self.steps_forward = 0;
        let settled = {
            let state = shared.lock();
            state.started && !state.seeking
        };

        // Kept frames show at once
        let shown = if settled {
            self.stepping
                .show_previous(&mut self.current, &mut self.pending, frames)
        } else {
            0
        };
        if shown > 0 {
            self.frame_serial += u64::from(shown);
            self.follow_shown(shared)?;
        }

        // The rest wait for the background decoder while it can fetch them
        let remaining = frames - shown;
        let waiting = remaining > 0 && settled && self.stepping.wait_back(remaining);
        self.plan_stepping(shared, info);
        if remaining == 0 || waiting && self.stepping.busy() {
            return Ok(());
        }

        // Without it, seek back and keep the frames decoded on the way
        self.stepping.stop_waiting();
        let shown = self
            .current
            .as_ref()
            .map(|frame| frame.position)
            .unwrap_or(shared.lock().clock.floor);
        let from = self.stepping.seek_target().unwrap_or(shown);
        let target = (from - info.frame_duration * f64::from(remaining)).max(0.);
        self.seek(target)?;
        self.stepping.seeking_back(target);

        Ok(())
    }

    /// Moves the clock and the audio to the shown frame after it changed while paused
    fn follow_shown(&self, shared: &Shared) -> Result<()> {
        let Some(position) = self.current.as_ref().map(|frame| frame.position) else {
            return Ok(());
        };

        let mut state = shared.lock();
        state.clock.anchor(position, Instant::now());
        state.realign_audio(position)?;
        drop(state);
        shared.notify();

        Ok(())
    }

    /// Starts the next background batch the user's steps call for, while paused
    fn plan_stepping(&mut self, shared: &Shared, info: StreamInfo) {
        if !paused_and_settled(&shared.lock()) {
            return;
        }

        if let Some(current) = &self.current {
            self.stepping
                .plan(current, info.frame_duration, self.steps_forward);
        }
    }

    /// Opens the background decoder, decodes until `deadline`, and shows what waiting
    /// steps can use
    pub fn background_work(&mut self, deadline: Instant) -> Background {
        let Some(shared) = self.shared.as_ref().map(Arc::clone) else {
            return Background::default();
        };

        let (info, settled) = {
            let state = shared.lock();
            (stream_info(&state), paused_and_settled(&state))
        };
        let Some(info) = info else {
            return Background::default();
        };

        // The decoder opens as soon as playback pauses, so the first step back finds it
        if settled && let (Some(path), Some(decoder)) = (&self.path, &self.decoder) {
            self.stepping.open(path, info, || decoder.open_sibling());
        }

        // Finished batches show at once for backward steps that were waiting
        let worked = self.stepping.work(
            deadline,
            &mut self.current,
            &mut self.pending,
            info.frame_duration,
        );
        if worked.stepped_back > 0 {
            self.frame_serial += u64::from(worked.stepped_back);
            if let Err(error) = self.follow_shown(&shared) {
                shared.fail(format!("{error:#}"));
            }
        }

        // The next batch starts as soon as the decoder is free
        if worked.finished || !self.stepping.busy() {
            self.plan_stepping(&shared, info);
        }

        Background {
            busy: self.stepping.busy(),
            changed: worked.stepped_back > 0 || worked.added_ahead,
        }
    }

    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        ensure!(seconds.is_finite(), "Seek position must be finite");

        let shared = self
            .shared
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No media is open"))?;
        let mut state = shared.lock();
        let info = state
            .stream
            .as_ref()
            .map(|stream| stream.info)
            .ok_or_else(|| anyhow::anyhow!("The media is still opening"))?;
        ensure!(info.duration > 0.0, "The media is not seekable");

        // The worker reads from the target, and the clock and audio wait there
        let target = seconds.clamp(0.0, info.duration);
        let floor = target.min((info.duration - info.frame_duration).max(0.0));
        state.generation += 1;
        state.seek = Some(Seek {
            target,
            generation: state.generation,
        });
        state.seeking = true;
        state.started = false;
        state.ended = false;
        state.eof = false;
        state.clock.seek(target, floor, Instant::now());
        state.packets.clear();
        state.feed.clear();
        shared.interrupt.seeking.store(true, Ordering::SeqCst);
        if let Some(audio) = &state.audio {
            audio.clear()?;
        }
        drop(state);

        // Keep current and its displayed texture until the requested frame replaces it
        shared.notify();
        self.stepping.forget_position();

        Ok(())
    }

    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        ensure!(volume.is_finite(), "Volume must be finite");
        self.volume = volume.clamp(0.0, 100.0);
        if let Some(shared) = &self.shared {
            let mut state = shared.lock();
            state.volume = self.volume;
            if let Some(audio) = &state.audio {
                audio.volume(self.volume)?;
            }
        }
        Ok(())
    }

    /// Changes playback speed from the current position, down to a standstill at zero
    ///
    /// The audio output stretches or resamples by the new rate from its next request,
    /// so nothing queued is dropped; the rate also applies to later videos
    pub fn set_rate(&mut self, rate: f64) -> Result<()> {
        ensure!(rate.is_finite(), "Playback rate must be finite");
        self.rate = rate.clamp(0.0, MAX_RATE);

        let Some(shared) = &self.shared else {
            return Ok(());
        };
        let mut state = shared.lock();
        if state.clock.rate() == self.rate {
            return Ok(());
        }

        state.set_rate(self.rate, Instant::now())?;
        drop(state);
        shared.notify();

        Ok(())
    }

    /// Chooses whether audio keeps its pitch or follows the playback speed
    pub fn set_pitch(&mut self, pitch: SpeedPitch) -> Result<()> {
        self.pitch = pitch;

        let Some(shared) = &self.shared else {
            return Ok(());
        };

        shared.lock().set_pitch(pitch)
    }

    /// Plays another audio stream, seeking to the current position so its packets are
    /// read from here
    pub fn set_audio_track(&mut self, index: usize) -> Result<()> {
        let shared = self
            .shared
            .as_ref()
            .map(Arc::clone)
            .context("No video is open")?;
        let position = {
            let mut state = shared.lock();
            ensure!(
                state.tracks.audio.iter().any(|track| track.index == index),
                "This video has no such audio track"
            );

            if state.audio_track == Some(index) {
                return Ok(());
            }

            state.audio_track = Some(index);
            state.position(Instant::now())?
        };

        self.seek(position)
    }

    /// Shows another subtitle track, or none
    ///
    /// Embedded tracks seek to the current position so their packets are read again;
    /// subtitle files were read whole when the video opened
    pub fn set_subtitle_track(&mut self, track: Option<usize>) -> Result<()> {
        let shared = self
            .shared
            .as_ref()
            .map(Arc::clone)
            .context("No video is open")?;
        let reread = {
            let mut state = shared.lock();
            let chosen = track.map(|index| {
                state
                    .tracks
                    .subtitles
                    .iter()
                    .find(|candidate| candidate.index == index)
                    .cloned()
                    .context("This video has no such subtitle track")
            });
            let chosen = chosen.transpose()?;

            self.subtitles_wanted = track.is_some();
            state.subtitles_wanted = self.subtitles_wanted;

            if state.subtitle_track == track {
                return Ok(());
            }

            state.subtitle_track = track;
            state.subtitles = state
                .external_subtitles
                .iter()
                .find(|subtitles| Some(subtitles.track.index) == track)
                .map(|subtitles| subtitles.cues.clone())
                .unwrap_or_default();

            let embedded = chosen.is_some_and(|chosen| !chosen.external);
            embedded
                .then(|| state.position(Instant::now()))
                .transpose()?
        };
        shared.notify();

        match reread {
            Some(position) if self.seekable(&shared) => self.seek(position),
            _ => Ok(()),
        }
    }

    fn seekable(&self, shared: &Shared) -> bool {
        stream_info(&shared.lock()).is_some_and(|info| info.duration > 0.)
    }

    pub fn set_decoder(&mut self, preference: VideoDecoder) -> Result<()> {
        let failed = self
            .shared
            .as_ref()
            .is_some_and(|shared| shared.stopped() || shared.lock().error.is_some());
        if self.decoder_preference == preference && !failed {
            return Ok(());
        }

        self.decoder_preference = preference;

        // A failed video reopens; one still opening picks the decoder up as it starts
        let Some(shared) = self.shared.as_ref().map(Arc::clone) else {
            return Ok(());
        };

        if failed {
            return self.reopen(&shared);
        }

        let stream = shared.lock().stream.as_ref().map(Arc::clone);
        let Some(stream) = stream.filter(|_| self.decoder.is_some()) else {
            return Ok(());
        };

        // A playing video seeks to where it is with the new decoder
        let position = shared.lock().position(Instant::now())?;
        self.seek(position).context(
            "Decoder preference saved; reopen this video to switch its decoder",
        )?;
        self.decoder = Some(DecoderSelection::new(
            Arc::clone(&self.gpu),
            stream,
            preference,
        ));
        self.pending = None;
        self.stepping.close();

        Ok(())
    }

    fn reopen(&mut self, shared: &Shared) -> Result<()> {
        let path = self.path.clone().context("No video is open")?;
        let resume = {
            let mut state = shared.lock();
            Resume {
                position: state.position(Instant::now()).unwrap_or_else(|_| {
                    self.current.as_ref().map_or(0., |frame| frame.position)
                }),
                paused: state.paused,
            }
        };
        self.open(&path)?;
        self.resume = Some(resume);
        Ok(())
    }

    pub fn poll(&mut self) -> Snapshot {
        let Some(shared) = self.shared.as_ref().map(Arc::clone) else {
            return self.idle_snapshot(None);
        };

        if let Err(error) = self.update(&shared) {
            shared.fail(format!("{error:#}"));
        }

        match self.snapshot(&shared) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                shared.fail(&error);
                self.idle_snapshot(Some(format!("{error:#}")))
            }
        }
    }

    fn update(&mut self, shared: &Shared) -> Result<()> {
        if self.prepare(shared)? {
            self.advance(shared)?;
        }

        self.pause_at_end(shared)
    }

    fn idle_snapshot(&self, error: Option<String>) -> Snapshot {
        Snapshot {
            video: VideoState {
                volume: self.volume,
                rate: self.rate,
                error,
                ..VideoState::default()
            },
            ..Snapshot::default()
        }
    }

    /// The frame to show and the stream it belongs to
    pub fn current_frame(&self) -> Option<(&DecodedFrame, Arc<Stream>)> {
        let current = self.current.as_ref()?;
        let stream = self
            .shared
            .as_ref()?
            .lock()
            .stream
            .as_ref()
            .map(Arc::clone)?;

        Some((&current.frame, stream))
    }

    /// Handles a current frame that could not be drawn
    ///
    /// A frame the renderer cannot import switches to the next decoder instead of failing
    pub fn render_failed(
        &mut self,
        error: anyhow::Error,
        unsupported_frame: bool,
    ) -> Result<()> {
        let Some(shared) = self.shared.as_ref().map(Arc::clone) else {
            return Err(error);
        };

        let result = if unsupported_frame && self.can_fallback() {
            // The frame can never be shown, so wait for the next decoder's frames
            self.current = None;
            self.recover_decoder(&shared, error)
        } else {
            Err(error)
        };

        if let Err(error) = &result {
            shared.fail(format!("{error:#}"));
        }
        result
    }

    fn can_fallback(&self) -> bool {
        self.decoder
            .as_ref()
            .is_some_and(DecoderSelection::can_fallback)
    }

    fn prepare(&mut self, shared: &Shared) -> Result<bool> {
        let mut state = shared.lock();
        if state.error.is_some() {
            return Ok(false);
        }

        let Some(stream) = state.stream.as_ref().map(Arc::clone) else {
            return Ok(false);
        };

        // A reopened video returns to where it was
        if let Some(resume) = self.resume.take() {
            state.paused = resume.paused;
            drop(state);
            if stream.info.duration > 0. {
                self.seek(resume.position)?;
            }
            return Ok(false);
        }

        // Audio and the decoder open once the worker knows the streams
        if state.audio_info.is_some() && state.audio.is_none() && !state.audio_disabled
        {
            open_audio(&mut state);
            shared.notify();
        }
        if self.decoder.is_none() {
            self.last_pts = -stream.info.frame_duration;
            self.decoder = Some(DecoderSelection::new(
                Arc::clone(&self.gpu),
                Arc::clone(&stream),
                self.decoder_preference,
            ));
        }
        if state.seeking {
            return Ok(false);
        }

        // A finished seek restarts decoding from its keyframe
        if state.flushed_generation != self.applied_generation {
            if let Some(decoder) = &mut self.decoder {
                decoder.flush();
            }
            self.pending = None;
            self.stepping.clear_frames();
            self.last_pts = state.clock.floor - stream.info.frame_duration;
            self.applied_generation = state.flushed_generation;
        }

        Ok(true)
    }

    fn advance(&mut self, shared: &Shared) -> Result<()> {
        let info = match shared.lock().stream.as_ref() {
            Some(stream) => stream.info,
            None => return Ok(()),
        };

        for _ in 0..MAX_FRAMES_PER_POLL {
            // Kept frames ahead come before anything the decoder has
            if self.pending.is_none() {
                self.pending = self.stepping.next_ahead();
            }

            // Past dropped frames, frames ahead come from the background decoder until
            // playback resumes, or from a seek if that decoder failed
            if self.pending.is_none()
                && let Some(gap) = self.stepping.gap()
            {
                if self.stepping.stranded() && self.steps_forward > 0 {
                    self.steps_forward -= 1;
                    self.seek(gap)?;
                }
                return Ok(());
            }

            // Otherwise the decoder supplies the next frame
            if self.pending.is_none() {
                let Some(decoder) = &mut self.decoder else {
                    return Ok(());
                };

                let frame = match decoder.next(shared) {
                    Ok(Some(frame)) => frame,
                    Ok(None) => return Ok(()),
                    Err(error) => {
                        self.recover_decoder(shared, error)?;
                        return Ok(());
                    }
                };
                let position = ffmpeg::timestamp(
                    frame.info().timestamp,
                    info.video_time_base,
                    info.origin,
                )
                .unwrap_or(self.last_pts + info.frame_duration);
                ensure!(position.is_finite(), "Invalid video timestamp");
                self.last_pts = position;
                self.pending = Some(TimedFrame { frame, position });
            }

            // The frame shows when a seek lands, a step asks, or the clock reaches it
            let Some(pending) = &self.pending else {
                return Ok(());
            };

            let mut state = shared.lock();
            let now = state.position(Instant::now())?;
            let starting = !state.started;
            let stepping = !starting && state.paused && self.steps_forward > 0;

            // Frames before a seek's target only lead up to it, unless a backward step's
            // seek keeps them
            if pending.position + info.frame_duration * 0.5 < state.clock.floor {
                if let Some(early) = self.pending.take()
                    && self.stepping.backfilling()
                {
                    self.stepping.remember(early);
                }
                continue;
            }

            if !starting
                && !stepping
                && (state.paused || pending.position > now + 0.002)
            {
                return Ok(());
            }

            // Show it, keeping the frame it replaces unless a seek came between them
            let position = pending.position;
            let previous = std::mem::replace(&mut self.current, self.pending.take());
            self.frame_serial += 1;
            if let Some(previous) = previous
                && !starting
            {
                self.stepping.remember(previous);
            }

            // A step moves the clock and the audio to the frame, so resuming continues here
            if stepping {
                self.steps_forward -= 1;
                state.clock.anchor(position, Instant::now());
                state.realign_audio(position)?;
            }

            // A landed seek starts the clock, and the audio unless paused
            if starting {
                self.stepping.seek_done();
                let position = position.max(state.clock.floor);
                state.clock.anchor(position, Instant::now());
                state.started = true;
                if let Some(audio) = &state.audio
                    && !state.paused
                {
                    audio.pause(false)?;
                }
            }
        }

        Ok(())
    }

    /// Restarts decoding at the current position with the next decoder candidate
    fn recover_decoder(&mut self, shared: &Shared, error: anyhow::Error) -> Result<()> {
        let Some(failed) = self
            .decoder
            .as_ref()
            .filter(|decoder| decoder.can_fallback())
            .and_then(DecoderSelection::active)
        else {
            return Err(error);
        };

        // The next decoder starts from a keyframe at the current position
        let position = shared.lock().position(Instant::now())?;
        self.seek(position)?;
        log::warn!(
            "{} video decoding failed: {error:#}. Trying the next decoder",
            failed.as_ref()
        );
        if let Some(decoder) = &mut self.decoder {
            decoder.fall_back(&error)?;
        }
        self.pending = None;
        self.stepping.close();

        Ok(())
    }

    /// Pauses on the last frame once decoding has finished and the clock reaches the end
    fn pause_at_end(&self, shared: &Shared) -> Result<()> {
        let mut state = shared.lock();
        let decoded_everything =
            self.decoder.as_ref().is_some_and(|decoder| decoder.eof)
                && self.pending.is_none();

        if !decoded_everything || !state.started || state.ended {
            return Ok(());
        }

        let end = self.end_position(stream_info(&state));
        if state.position(Instant::now())? < end {
            return Ok(());
        }

        state.ended = true;
        state.paused = true;
        state.clock.anchor(end, Instant::now());

        if let Some(audio) = &state.audio {
            audio.pause(true)?;
        }

        Ok(())
    }

    /// The media duration, or the end of the current frame when the container has none
    fn end_position(&self, info: Option<StreamInfo>) -> f64 {
        let duration = info.map_or(0.0, |info| info.duration);

        if duration > 0.0 {
            return duration;
        }

        let frame_duration = info.map_or(0.0, |info| info.frame_duration);
        self.current
            .as_ref()
            .map_or(0.0, |frame| frame.position + frame_duration)
    }

    fn snapshot(&self, shared: &Shared) -> Result<Snapshot> {
        let mut state = shared.lock();
        let info = stream_info(&state);
        let duration = info.map_or(0.0, |info| info.duration);

        let mut position = if state.ended {
            self.end_position(info)
        } else {
            state.position(Instant::now())?
        };
        if duration > 0.0 {
            position = position.min(duration);
        }

        let rate = state.clock.rate();
        let next_frame_delay = match &self.pending {
            // A stopped clock never reaches the next frame
            Some(_) if rate < SILENT_RATE => 0.1,
            Some(frame) => (frame.position - position).max(0.0) / rate,
            None => 0.005,
        };
        let size = self.current.as_ref().map_or([0; 2], |frame| {
            frame
                .frame
                .display_size(info.map_or(0.0, |info| info.rotation))
        });

        let video = VideoState {
            active: true,
            loading: self.current.is_none() && state.error.is_none(),
            seeking: self.current.is_some() && (!state.started || state.seeking),
            paused: state.paused,
            position,
            duration,
            volume: state.volume,
            rate,
            pitch: self.pitch,
            size,
            decoder: self.decoder.as_ref().and_then(|decoder| decoder.selected),
            fallback: self.decoder.as_ref().and_then(|decoder| decoder.fallback),
            software_reason: self
                .decoder
                .as_ref()
                .and_then(|decoder| decoder.software_reason.clone()),
            codec: info.map_or("", |info| info.codec),
            audio: state.audio_info,
            tracks: state.tracks.clone(),
            audio_track: state.audio_track,
            subtitle_track: state.subtitle_track,
            subtitles: if state.subtitle_track.is_some() {
                state.subtitles.active(position)
            } else {
                ActiveSubtitles::default()
            },
            error: state.error.clone(),
        };

        Ok(Snapshot {
            video,
            next_frame_delay,
            frame_serial: self.frame_serial,
            audio_output: state
                .audio
                .as_ref()
                .map_or_else(String::new, |audio| audio.name.clone()),
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn stream_info(state: &PlaybackState) -> Option<StreamInfo> {
    state.stream.as_ref().map(|stream| stream.info)
}

/// Whether playback is paused on a shown frame, with no seek under way
fn paused_and_settled(state: &PlaybackState) -> bool {
    state.paused && state.started && !state.seeking
}

/// Opens the output device, or falls back to silent playback on the wall clock
fn open_audio(state: &mut PlaybackState) {
    match AudioOutput::open(state.volume, Arc::clone(&state.feed)) {
        Ok(audio) => {
            state.audio = Some(audio);

            if let Err(error) = state.apply_ratio() {
                log::warn!("Cannot apply the playback speed to audio: {error:#}");
            }
        }
        Err(error) => {
            log::warn!("Playing without sound: {error:#}");
            state.audio_disabled = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, ensure};

    use super::*;
    use crate::video::{
        gpu,
        playback::test_support::{chosen_decoder, test_graphics},
    };

    #[test]
    #[ignore = "Requires Vulkan, a hardware H.264 decoder, and ffmpeg with libx264"]
    fn hardware_failure_recovers_with_the_next_decoder() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = folder.path().join("recovery.mp4");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24",
                "-t",
                "3",
                "-c:v",
                "libx264",
            ])
            .arg(&clip)
            .status()?;
        ensure!(
            generated.success(),
            "Could not generate the recovery fixture"
        );

        let graphics = test_graphics()?;
        let gpu = gpu::for_device(&graphics.device)?;
        let mut engine = Engine::new(gpu, egui::Context::default());
        engine.open(&clip)?;
        let state = pump(&mut engine, |state| state.video.position > 0.1)?.video;
        let hardware = state
            .decoder
            .filter(|decoder| *decoder != VideoDecoder::Software)
            .context("A hardware decoder is required for this test")?;
        engine.toggle_pause()?;
        let position = state.position;
        let shared = engine
            .shared
            .as_ref()
            .map(Arc::clone)
            .context("No media worker")?;
        engine
            .recover_decoder(&shared, anyhow::anyhow!("Injected hardware failure"))?;
        ensure!(
            engine.current.is_some(),
            "Recovery discarded the current frame"
        );

        let state = pump(&mut engine, |state| {
            state
                .video
                .decoder
                .is_some_and(|decoder| decoder != hardware)
                && !state.video.seeking
        })?
        .video;
        ensure!(state.paused, "Recovery changed pause state");
        ensure!(
            (state.position - position).abs() < 0.15,
            "Recovery lost the playback position"
        );
        ensure!(state.size == [320, 180], "Recovery lost frame dimensions");
        ensure!(
            state
                .fallback
                .is_some_and(|fallback| fallback.from == hardware),
            "The decoder switch was not reported"
        );

        // A terminal import failure stops the demux worker and latches the UI error
        shared.fail("Injected frame import failure");
        let state = engine.poll();
        ensure!(state.video.error.is_some(), "Failure was not reported");
        ensure!(shared.stopped(), "The failed worker was not stopped");

        engine.set_decoder(hardware)?;
        let state = pump(&mut engine, |state| {
            state.video.decoder == Some(hardware) && !state.video.seeking
        })?
        .video;
        ensure!(state.paused, "Reopening changed pause state");
        ensure!(
            (state.position - position).abs() < 0.15,
            "Reopening lost the playback position"
        );
        ensure!(state.audio.is_none(), "A silent clip reported audio");
        engine.stop();
        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn frame_steps_move_one_frame_and_playback_resumes_from_there() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = clip_with_audio(folder.path(), 10)?;
        let mut engine = audible_engine(&clip)?;
        pump(&mut engine, |state| state.video.position > 0.5)?;

        engine.toggle_pause()?;
        let start = shown(&engine)?;

        // Twelve steps outlast the audio queue, which must not stop video packets
        for step in 1..=12 {
            engine.step(1)?;
            let expected = start + f64::from(step) * 0.1;
            pump_engine(&mut engine, |engine, _| near(engine, expected))?;
        }
        let state = engine.poll().video;
        ensure!(state.paused, "Stepping resumed playback");
        ensure!(
            (state.position - (start + 1.2)).abs() < 0.02,
            "The clock did not follow the stepped frame"
        );

        engine.step(-1)?;
        pump_engine(&mut engine, |engine, state| {
            near(engine, start + 1.1) && !state.video.seeking
        })?;

        // A second backward step before the first lands still moves back two frames
        engine.step(-1)?;
        engine.step(-1)?;
        pump_engine(&mut engine, |engine, state| {
            near(engine, start + 0.9) && !state.video.seeking
        })?;

        engine.step(1)?;
        pump_engine(&mut engine, |engine, _| near(engine, start + 1.0))?;
        engine.toggle_pause()?;
        let resumed = pump(&mut engine, |state| {
            !state.video.paused && state.video.position > start + 1.5
        })?;
        ensure!(
            resumed.video.position < start + 3.,
            "Playback did not resume from the stepped frame"
        );
        let heard = heard(&engine).context("No audio after resuming")?;
        ensure!(
            (heard - resumed.video.position).abs() < 0.1,
            "Audio resumed at {heard}, away from the picture"
        );

        engine.stop();
        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn stepping_back_reuses_kept_frames_without_decoding_again() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = clip_with_audio(folder.path(), 10)?;
        let mut engine = audible_engine(&clip)?;
        pump(&mut engine, |state| state.video.position > 3.)?;

        engine.toggle_pause()?;
        let start = shown(&engine)?;

        // Frames shown while playing come back on the very next poll, up to the few
        // kept during playback
        for step in 1..=6 {
            engine.step(-1)?;
            let state = engine.poll();
            let expected = start - f64::from(step) * 0.1;
            ensure!(
                near(&engine, expected) && !state.video.seeking,
                "Step back {step} was not instant: showing {:?}",
                shown(&engine).ok()
            );
        }

        // Stepping forward again replays them in order
        for step in 1..=3 {
            engine.step(1)?;
            let expected = start - f64::from(6 - step) * 0.1;
            pump_engine(&mut engine, |engine, _| near(engine, expected))?;
        }

        // After a seek nothing is kept; the first step waits for a background batch,
        // which covers the next ones
        engine.seek(5.95)?;
        pump_engine(&mut engine, |engine, state| {
            near(engine, 5.9) || near(engine, 6.) && !state.video.seeking
        })?;
        let landed = shown(&engine)?;
        engine.step(-1)?;
        pump_engine(&mut engine, |engine, state| {
            near(engine, landed - 0.1) && !state.video.seeking
        })?;

        for step in 2..=6 {
            engine.step(-1)?;
            engine.poll();
            let expected = landed - f64::from(step) * 0.1;
            ensure!(
                near(&engine, expected),
                "Step back {step} after a seek was not instant: showing {:?}",
                shown(&engine).ok()
            );
        }

        engine.stop();
        Ok(())
    }

    /// One turn of the playback thread: a slice of background decoding, then a poll
    fn tick(engine: &mut Engine) -> Result<Snapshot> {
        engine.background_work(Instant::now() + Duration::from_millis(3));
        let state = engine.poll();
        ensure!(
            state.video.error.is_none(),
            "Playback failed: {:?}",
            state.video.error
        );
        std::thread::sleep(Duration::from_millis(1));

        Ok(state)
    }

    /// Runs the playback thread's turns for `duration`, as between key repeats
    fn idle(engine: &mut Engine, duration: Duration) -> Result<()> {
        let end = Instant::now() + duration;
        while Instant::now() < end {
            tick(engine)?;
        }
        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn held_backward_steps_never_wait_and_forward_steps_continue_in_order() -> Result<()>
    {
        // Keyframes two seconds apart, on the decoder and at the size under test
        let folder = tempfile::tempdir()?;
        let size = std::env::var("FLOWA_TEST_CLIP_SIZE");
        let size = size.as_deref().unwrap_or("160x90");
        let clip = clip_with_audio_sized(folder.path(), 30, size)?;
        let decoder = chosen_decoder().unwrap_or(VideoDecoder::Software);
        let mut engine = audible_engine_with(&clip, decoder)?;
        let playing = pump(&mut engine, |state| state.video.position > 5.)?;
        if decoder != VideoDecoder::Auto {
            ensure!(
                playing.video.decoder == Some(decoder),
                "Decoding used {:?} instead of {decoder:?}",
                playing.video.decoder
            );
        }

        // Pausing opens the background decoder before the first step
        engine.toggle_pause()?;
        idle(&mut engine, Duration::from_millis(100))?;
        let start = shown(&engine)?;
        let frame = 1. / 30.;

        // Ninety steps at key-repeat speed cross a keyframe two seconds apart
        let mut late = Vec::new();
        for step in 1..=90 {
            engine.step(-1)?;
            engine.poll();
            let expected = start - f64::from(step) * frame;
            if !near(&engine, expected) {
                late.push(step);
            }
            idle(&mut engine, Duration::from_millis(33))?;
            ensure!(
                near(&engine, expected),
                "Step {step} never reached {expected}: showing {:?}",
                shown(&engine).ok()
            );
        }
        ensure!(late.is_empty(), "Backward steps waited: {late:?}");

        // Forward steps replay the kept frames, then forward batches past dropped ones
        let back = shown(&engine)?;
        for step in 1..=40 {
            engine.step(1)?;
            let expected = back + f64::from(step) * frame;
            pump_engine(&mut engine, |engine, _| near(engine, expected))?;
        }

        // Playing continues from the shown frame
        let paused_at = shown(&engine)?;
        engine.toggle_pause()?;
        let played = pump(&mut engine, |state| state.video.position > paused_at + 0.5)?;
        ensure!(
            played.video.position < paused_at + 1.5,
            "Playback jumped to {} from {paused_at}",
            played.video.position
        );
        engine.stop();

        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn playback_rate_changes_smoothly_and_audio_drives_the_clock() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = clip_with_audio(folder.path(), 30)?;
        let mut engine = audible_engine(&clip)?;
        pump(&mut engine, |state| state.video.position > 0.3)?;

        for pitch in [SpeedPitch::Keep, SpeedPitch::Follow, SpeedPitch::Lower] {
            engine.set_pitch(pitch)?;

            for (rate, wall) in [(2.0, 1.2), (0.5, 1.6), (1.0, 0.6)] {
                let before = engine.poll();
                engine.set_rate(rate)?;
                let after = follow_smoothly(&mut engine, wall)?;
                let advanced = after.video.position - before.video.position;

                ensure!(
                    after.video.rate == rate,
                    "The snapshot reported another rate"
                );
                ensure!(
                    (advanced - wall * rate).abs() < 0.2,
                    "{pitch:?} at {rate}× moved {advanced} seconds in {wall}"
                );
                ensure!(
                    after.frame_serial
                        > before.frame_serial + (wall * 30. * rate * 0.8) as u64,
                    "Too few video frames at {rate}×"
                );

                let lag = after.video.position - shown(&engine)?;
                ensure!(lag < 0.15, "Video fell {lag} seconds behind at {rate}×");

                let heard =
                    heard(&engine).context("Audio was not driving the clock")?;
                ensure!(
                    (heard - after.video.position).abs() < 0.05,
                    "{pitch:?} at {rate}× heard {heard} while showing {}",
                    after.video.position
                );
            }
        }

        // A speed of zero holds the picture and the clock without pausing
        engine.set_rate(0.)?;
        let stopped = follow_smoothly(&mut engine, 0.3)?.video.position;
        let still = follow_smoothly(&mut engine, 0.5)?.video;
        ensure!(!still.paused, "A speed of zero paused playback");
        ensure!(
            (still.position - stopped).abs() < 0.01,
            "The clock moved at zero speed"
        );

        engine.set_rate(1.)?;
        let moving = follow_smoothly(&mut engine, 0.5)?.video.position;
        ensure!(
            moving > still.position + 0.35,
            "Playback did not resume from zero"
        );

        engine.stop();
        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn toggling_pause_quickly_at_low_speed_neither_rewinds_nor_freezes() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = clip_with_audio(folder.path(), 30)?;
        let mut engine = audible_engine(&clip)?;
        pump(&mut engine, |state| state.video.position > 0.5)?;

        for pitch in [SpeedPitch::Keep, SpeedPitch::Follow, SpeedPitch::Lower] {
            engine.set_pitch(pitch)?;
            engine.set_rate(0.25)?;
            let before = follow_smoothly(&mut engine, 0.6)?.video.position;

            for _ in 0..12 {
                engine.toggle_pause()?;
                follow_smoothly(&mut engine, 0.03)?;
            }
            let after = engine.poll().video;
            ensure!(
                !after.paused,
                "An even number of toggles left playback paused"
            );
            ensure!(
                after.position > before - 0.02,
                "{pitch:?}: toggling moved the playhead back from {before} to {}",
                after.position
            );

            let serial = engine.poll().frame_serial;
            let state = follow_smoothly(&mut engine, 0.6)?;
            ensure!(
                state.frame_serial >= serial + 3,
                "{pitch:?}: the picture froze after resuming"
            );

            let lag = state.video.position - shown(&engine)?;
            ensure!(
                lag < 0.1,
                "{pitch:?}: the picture fell {lag} seconds behind"
            );
            engine.set_rate(1.)?;
        }

        engine.stop();
        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn audible_video_pauses_at_its_end_at_any_pitch_setting() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = clip_with_audio(folder.path(), 30)?;
        let mut engine = audible_engine(&clip)?;

        for pitch in [SpeedPitch::Keep, SpeedPitch::Follow, SpeedPitch::Lower] {
            engine.set_pitch(pitch)?;
            engine.set_rate(1.5)?;
            let duration = pump(&mut engine, |state| state.video.duration > 0.)?
                .video
                .duration;

            engine.seek(duration - 1.)?;
            let ended = pump(&mut engine, |state| state.video.paused)?.video;
            ensure!(
                (ended.position - duration).abs() < 0.05,
                "{pitch:?}: paused at {} instead of the end",
                ended.position
            );

            // Playing again from the end starts over
            engine.toggle_pause()?;
        }

        engine.stop();
        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn low_frame_rate_video_keeps_up_with_its_audio() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = clip_with_audio(folder.path(), 10)?;
        let mut engine = audible_engine(&clip)?;
        pump(&mut engine, |state| state.video.position > 1.)?;

        let state = follow_smoothly(&mut engine, 1.5)?.video;
        let lag = state.position - shown(&engine)?;
        ensure!(lag < 0.15, "A 10 fps video fell {lag} seconds behind");

        engine.stop();
        Ok(())
    }

    /// Polls for `seconds` and fails if the clock ever jumps or runs backward
    fn follow_smoothly(engine: &mut Engine, seconds: f64) -> Result<Snapshot> {
        let until = Instant::now() + Duration::from_secs_f64(seconds);
        let mut previous = engine.poll();

        loop {
            std::thread::sleep(Duration::from_millis(5));
            let state = engine.poll();
            let step = state.video.position - previous.video.position;
            ensure!(
                (-0.01..0.08).contains(&step),
                "The clock jumped by {step} at {}",
                state.video.position
            );
            ensure!(state.video.error.is_none(), "{:?}", state.video.error);

            if Instant::now() > until {
                return Ok(state);
            }
            previous = state;
        }
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn switching_audio_tracks_keeps_the_position_and_reports_the_track() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = folder.path().join("tracks.mkv");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:sample_rate=48000",
                "-map",
                "0",
                "-map",
                "1",
                "-map",
                "2",
                "-t",
                "8",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-ac:a:0",
                "2",
                "-ac:a:1",
                "1",
                "-metadata:s:a:0",
                "language=eng",
                "-metadata:s:a:1",
                "language=jpn",
                "-metadata:s:a:1",
                "title=Commentary",
            ])
            .arg(&clip)
            .status()?;
        ensure!(generated.success(), "Could not generate the tracks fixture");

        let mut engine = audible_engine(&clip)?;
        let state = pump(&mut engine, |state| state.video.position > 1.)?.video;
        let labels: Vec<_> = state
            .tracks
            .audio
            .iter()
            .map(|t| t.label.as_str())
            .collect();
        ensure!(
            labels == ["English · AAC stereo", "Commentary · Japanese · AAC mono"],
            "Unexpected track labels {labels:?}"
        );

        let first = state.tracks.audio[0].index;
        let second = state.tracks.audio[1].index;
        ensure!(
            state.audio_track == Some(first),
            "The first track did not play"
        );

        let position = state.position;
        engine.set_audio_track(second)?;
        let switched = pump(&mut engine, |state| {
            state.video.audio_track == Some(second)
                && !state.video.seeking
                && state.video.audio.is_some_and(|audio| audio.channels == 1)
        })?
        .video;
        ensure!(
            (switched.position - position).abs() < 0.5,
            "Switching moved playback from {position} to {}",
            switched.position
        );

        let later = switched.position + 0.5;
        pump(&mut engine, |state| state.video.position > later)?;
        ensure!(heard(&engine).is_some(), "The new track did not play");

        engine.stop();
        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn embedded_and_external_subtitles_show_at_their_times() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let embedded = folder.path().join("embedded.srt");
        std::fs::write(
            &embedded,
            "1\n00:00:00,500 --> 00:00:03,000\nHello <i>there</i>\n\n\
             2\n00:00:04,000 --> 00:00:06,000\nSecond line\n",
        )?;
        let clip = folder.path().join("captioned.mkv");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=30",
            ])
            .arg("-i")
            .arg(&embedded)
            .args([
                "-t",
                "8",
                "-c:v",
                "libx264",
                "-c:s",
                "srt",
                "-metadata:s:s:0",
                "language=eng",
                "-disposition:s:0",
                "default",
            ])
            .arg(&clip)
            .status()?;
        ensure!(
            generated.success(),
            "Could not generate the subtitle fixture"
        );
        std::fs::write(
            folder.path().join("captioned.fr.srt"),
            "1\n00:00:01,000 --> 00:00:05,000\nBonjour\n",
        )?;

        let graphics = test_graphics()?;
        let gpu = gpu::for_device(&graphics.device)?;
        let mut engine = Engine::new(gpu, egui::Context::default());
        engine.set_decoder(VideoDecoder::Software)?;
        engine.open(&clip)?;

        let state =
            pump(&mut engine, |state| state.video.tracks.subtitles.len() == 2)?.video;
        let labels: Vec<_> = state
            .tracks
            .subtitles
            .iter()
            .map(|t| t.label.as_str())
            .collect();
        ensure!(
            labels == ["English · SRT", "French · SRT · External"],
            "Unexpected subtitle labels {labels:?}"
        );
        let [embedded_track, external_track] =
            [0, 1].map(|i| state.tracks.subtitles[i].index);
        ensure!(
            state.subtitle_track == Some(embedded_track),
            "The default subtitle track was not chosen"
        );

        let state = pump(&mut engine, |state| state.video.position > 1.)?.video;
        ensure!(
            cue_text(&state) == ["Hello there"],
            "Unexpected cues {:?}",
            cue_text(&state)
        );

        engine.set_subtitle_track(Some(external_track))?;
        let state = engine.poll().video;
        ensure!(
            state.position < 3. && cue_text(&state) == ["Bonjour"],
            "The subtitle file did not show at {}: {:?}",
            state.position,
            cue_text(&state)
        );

        engine.set_subtitle_track(Some(embedded_track))?;
        let state = pump(&mut engine, |state| state.video.position > 4.2)?.video;
        ensure!(
            cue_text(&state) == ["Second line"],
            "Switching back did not show the embedded cue: {:?}",
            cue_text(&state)
        );

        engine.set_subtitle_track(None)?;
        ensure!(
            engine.poll().video.subtitles.is_empty(),
            "Subtitles stayed on"
        );

        engine.stop();
        Ok(())
    }

    fn cue_text(state: &VideoState) -> Vec<String> {
        state
            .subtitles
            .iter()
            .filter_map(|cue| match &cue.content {
                crate::video::subtitles::CueContent::Text(text) => Some(
                    text.spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>(),
                ),
                crate::video::subtitles::CueContent::Bitmap(_) => None,
            })
            .collect()
    }

    fn clip_with_audio(
        folder: &std::path::Path,
        frame_rate: u32,
    ) -> Result<std::path::PathBuf> {
        clip_with_audio_sized(folder, frame_rate, "160x90")
    }

    fn clip_with_audio_sized(
        folder: &std::path::Path,
        frame_rate: u32,
        size: &str,
    ) -> Result<std::path::PathBuf> {
        let clip = folder.join("audible.mp4");
        let video = format!("testsrc2=size={size}:rate={frame_rate}");
        let keyframes = (frame_rate * 2).to_string();
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                &video,
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "20",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-g",
                &keyframes,
                "-c:a",
                "aac",
            ])
            .arg(&clip)
            .status()?;
        ensure!(
            generated.success(),
            "Could not generate the audible fixture"
        );

        Ok(clip)
    }

    /// A software-decoding engine that has opened an audio device for the clip
    fn audible_engine(clip: &std::path::Path) -> Result<Engine> {
        audible_engine_with(clip, VideoDecoder::Software)
    }

    fn audible_engine_with(
        clip: &std::path::Path,
        decoder: VideoDecoder,
    ) -> Result<Engine> {
        // SDL keeps the audio subsystem initialized for the rest of the test process
        std::mem::forget(crate::video::audio::AudioRuntime::new()?);
        let graphics = test_graphics()?;
        let gpu = gpu::for_device(&graphics.device)?;
        let mut engine = Engine::new(gpu, egui::Context::default());
        engine.set_decoder(decoder)?;
        engine.open(clip)?;
        pump(&mut engine, |state| state.video.position > 0.1)?;

        // These tests need the audio clock, so a missing device fails them clearly
        let audible = engine
            .shared
            .as_ref()
            .is_some_and(|shared| shared.lock().audio.is_some());
        ensure!(
            audible,
            "No audio output opened; set SDL_AUDIO_DRIVER=dummy"
        );

        Ok(engine)
    }

    /// The position of the audio being heard, which drives the clock
    fn heard(engine: &Engine) -> Option<f64> {
        let shared = engine.shared.as_ref()?;
        let state = shared.lock();
        let audio = state.audio.as_ref()?;

        state
            .feed
            .heard_position(audio.queued_bytes().ok()?, audio.latency)
    }

    fn shown(engine: &Engine) -> Result<f64> {
        engine
            .current
            .as_ref()
            .map(|frame| frame.position)
            .context("No frame is shown")
    }

    fn near(engine: &Engine, position: f64) -> bool {
        shown(engine).is_ok_and(|shown| (shown - position).abs() < 0.02)
    }

    fn pump_engine(
        engine: &mut Engine,
        ready: impl Fn(&Engine, &Snapshot) -> bool,
    ) -> Result<Snapshot> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = tick(engine)?;
            if ready(engine, &state) {
                return Ok(state);
            }

            ensure!(
                Instant::now() < deadline,
                "Timed out at {:?}",
                shown(engine).ok()
            );
        }
    }

    fn pump(
        engine: &mut Engine,
        ready: impl Fn(&Snapshot) -> bool,
    ) -> Result<Snapshot> {
        pump_engine(engine, |engine, state| {
            ready(state) && engine.current.is_some()
        })
    }
}
