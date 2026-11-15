//! Playback lifecycle, controls, frame scheduling, and ownership of the media worker

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    thread::JoinHandle,
    time::Instant,
};

use anyhow::{Context, Result, ensure};

use super::{
    state::{PlaybackState, Seek, Shared},
    worker,
};
use crate::settings::VideoDecoder;
use crate::video::{
    PLAYBACK_RATES, VideoState,
    audio::AudioOutput,
    decoding::{DecodedFrame, DecoderSelection},
    ffmpeg::{self, Stream, StreamInfo},
    gpu::NativeGpu,
};

/// Bounds how many late frames one poll may skip while catching up to the clock
const MAX_FRAMES_PER_POLL: usize = 12;
/// Bounds forward steps waiting for frames, so a held key never runs far ahead
const MAX_QUEUED_STEPS: u32 = 8;

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

struct TimedFrame {
    frame: DecodedFrame,
    position: f64,
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
    pending: Option<TimedFrame>,
    applied_generation: u64,
    frame_serial: u64,
    last_pts: f64,
    volume: f64,
    rate: f64,
    /// Frames still to show while paused, one per forward step
    steps_forward: u32,
    /// Where the latest backward step is seeking, so repeated steps keep going back
    step_back_target: Option<f64>,
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
            applied_generation: 0,
            frame_serial: 0,
            last_pts: 0.0,
            volume: 100.0,
            rate: 1.0,
            steps_forward: 0,
            step_back_target: None,
        }
    }

    pub fn open(&mut self, path: &Path) -> Result<()> {
        self.stop();
        let shared = Arc::new(Shared::new(self.ctx.clone(), self.volume, self.rate));
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
        self.decoder = None;
        self.frame_serial = 0;
        self.applied_generation = 0;
        self.last_pts = 0.0;
        self.steps_forward = 0;
        self.step_back_target = None;

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
        let mut state = shared.lock();
        let now = Instant::now();
        let position = state.position(now)?;
        state.clock.anchor(position, now);
        state.paused = paused;
        let restart = state.ended && !paused;
        let realign = state.holding_audio && !paused;
        if let Some(audio) = &state.audio {
            audio.pause(paused || !state.started)?;
        }
        drop(state);
        shared.notify();
        self.steps_forward = 0;

        if restart {
            self.seek(0.0)?;
        } else if realign {
            // Queued audio belongs to where stepping started, so refill it from here
            self.seek(position)?;
        }
        Ok(())
    }

    /// Pauses and shows the next frame, or seeks back to the previous one
    pub fn step(&mut self, frames: i32) -> Result<()> {
        let Some(shared) = self.shared.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        let Some(info) = stream_info(&shared.lock()) else {
            return Ok(());
        };

        self.set_paused(true)?;

        if frames < 0 {
            return self.step_back(&shared, info, frames.unsigned_abs());
        }

        let mut state = shared.lock();
        if state.ended {
            return Ok(());
        }

        state.holding_audio = true;
        self.steps_forward = (self.steps_forward + frames as u32).min(MAX_QUEUED_STEPS);
        drop(state);
        shared.notify();

        Ok(())
    }

    /// Seeks one frame duration back per step from the shown frame, or from a backward
    /// step that is still seeking
    fn step_back(
        &mut self,
        shared: &Shared,
        info: StreamInfo,
        frames: u32,
    ) -> Result<()> {
        let shown = self
            .current
            .as_ref()
            .map(|frame| frame.position)
            .unwrap_or(shared.lock().clock.floor);
        let from = self.step_back_target.unwrap_or(shown);
        let target = (from - info.frame_duration * f64::from(frames)).max(0.);

        self.steps_forward = 0;
        self.seek(target)?;
        self.step_back_target = Some(target);

        Ok(())
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
        state.holding_audio = false;
        state.clock.seek(target, floor, Instant::now());
        state.packets.clear();
        shared.interrupt.seeking.store(true, Ordering::SeqCst);
        if let Some(audio) = &state.audio {
            audio.clear()?;
        }
        drop(state);

        // Keep current and its displayed texture until the requested frame replaces it
        shared.notify();
        self.step_back_target = None;
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

    /// Changes playback speed from the current position, keeping the pitch of audio
    ///
    /// Audio already queued was stretched for the old rate, so it is dropped and the
    /// worker refills the queue; the rate also applies to later videos
    pub fn set_rate(&mut self, rate: f64) -> Result<()> {
        ensure!(
            rate.is_finite() && rate > 0.0,
            "Playback rate must be positive"
        );
        let slowest = PLAYBACK_RATES[0];
        let fastest = PLAYBACK_RATES[PLAYBACK_RATES.len() - 1];
        self.rate = rate.clamp(slowest, fastest);

        let Some(shared) = &self.shared else {
            return Ok(());
        };
        let mut state = shared.lock();
        if state.clock.rate() == self.rate {
            return Ok(());
        }

        let now = Instant::now();
        let position = state.position(now)?;
        state.clock.set_rate(self.rate, position, now);
        state.rate_epoch += 1;

        if let Some(audio) = &state.audio {
            audio.clear()?;
            audio.pause(state.paused || !state.started)?;

            if state.clock.audio_end.is_some() {
                state.clock.audio_end = Some(position);
            }
        }
        drop(state);
        shared.notify();

        Ok(())
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

        if let Some(resume) = self.resume.take() {
            state.paused = resume.paused;
            drop(state);
            if stream.info.duration > 0. {
                self.seek(resume.position)?;
            }
            return Ok(false);
        }

        if stream.info.audio.is_some() && state.audio.is_none() && !state.audio_disabled
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

        if state.flushed_generation != self.applied_generation {
            if let Some(decoder) = &mut self.decoder {
                decoder.flush();
            }
            self.pending = None;
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

            let Some(pending) = &self.pending else {
                return Ok(());
            };
            let mut state = shared.lock();
            let now = state.position(Instant::now())?;
            let starting = !state.started;
            let stepping = !starting && state.paused && self.steps_forward > 0;

            if pending.position + info.frame_duration * 0.5 < state.clock.floor {
                self.pending = None;
                continue;
            }
            if !starting
                && !stepping
                && (state.paused || pending.position > now + 0.002)
            {
                return Ok(());
            }

            let position = pending.position;
            self.current = self.pending.take();
            self.frame_serial += 1;
            if stepping {
                self.steps_forward -= 1;
                state.clock.anchor(position, Instant::now());
            }
            if starting {
                self.step_back_target = None;
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
        let next_frame_delay = self
            .pending
            .as_ref()
            .map_or(0.005, |frame| (frame.position - position).max(0.0) / rate);
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
            size,
            decoder: self.decoder.as_ref().and_then(|decoder| decoder.selected),
            fallback: self.decoder.as_ref().and_then(|decoder| decoder.fallback),
            software_reason: self
                .decoder
                .as_ref()
                .and_then(|decoder| decoder.software_reason.clone()),
            codec: info.map_or("", |info| info.codec),
            audio: info.and_then(|info| info.audio),
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

/// Opens the output device, or falls back to silent playback on the wall clock
fn open_audio(state: &mut PlaybackState) {
    match AudioOutput::open(state.volume) {
        Ok(audio) => state.audio = Some(audio),
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
    use eframe::egui_wgpu::{RenderState, RendererOptions, WgpuConfiguration};

    use super::*;
    use crate::video::gpu;

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

        let setup = gpu::setup()?;
        let instance = pollster::block_on(setup.new_instance());
        let config = WgpuConfiguration {
            wgpu_setup: setup,
            ..Default::default()
        };
        let graphics = pollster::block_on(RenderState::create(
            &config,
            &instance,
            None,
            RendererOptions::default(),
        ))?;
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
        ensure!(!holding_audio(&engine), "Resuming kept dropping audio");

        engine.stop();
        Ok(())
    }

    #[test]
    #[ignore = "Requires Vulkan and ffmpeg with libx264"]
    fn playback_rate_scales_the_clock_while_audio_keeps_playing() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = clip_with_audio(folder.path(), 30)?;
        let mut engine = audible_engine(&clip)?;
        pump(&mut engine, |state| state.video.position > 0.3)?;

        for (rate, wall) in [(2.0, 1.5), (0.5, 2.0)] {
            engine.set_rate(rate)?;

            // Let the worker refill the audio queue that the change dropped
            let settled = Instant::now() + Duration::from_millis(300);
            pump_engine(&mut engine, |_, _| Instant::now() > settled)?;

            let before = engine.poll();
            let serial = before.frame_serial;
            let until = Instant::now() + Duration::from_secs_f64(wall);
            let after = pump_engine(&mut engine, |_, _| Instant::now() > until)?;
            let advanced = after.video.position - before.video.position;

            ensure!(
                after.video.rate == rate,
                "The snapshot reported another rate"
            );
            ensure!(
                (advanced - wall * rate).abs() < 0.25,
                "At {rate}× the clock moved {advanced} seconds in {wall}"
            );
            ensure!(
                after.frame_serial > serial + (wall * 30. * rate * 0.8) as u64,
                "Too few video frames at {rate}×"
            );
            let lag = after.video.position - shown(&engine)?;
            ensure!(lag < 0.15, "Video fell {lag} seconds behind at {rate}×");
            ensure!(audio_queued(&engine), "Audio stopped at {rate}×");
        }

        engine.stop();
        Ok(())
    }

    fn clip_with_audio(
        folder: &std::path::Path,
        frame_rate: u32,
    ) -> Result<std::path::PathBuf> {
        let clip = folder.join("audible.mp4");
        let video = format!("testsrc2=size=160x90:rate={frame_rate}");
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
                "12",
                "-c:v",
                "libx264",
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
        // SDL keeps the audio subsystem initialized for the rest of the test process
        std::mem::forget(crate::video::audio::AudioRuntime::new()?);

        let graphics = test_graphics()?;
        let gpu = gpu::for_device(&graphics.device)?;
        let mut engine = Engine::new(gpu, egui::Context::default());
        engine.set_decoder(VideoDecoder::Software)?;
        engine.open(clip)?;
        pump(&mut engine, |state| state.video.position > 0.1)?;

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

    fn holding_audio(engine: &Engine) -> bool {
        engine
            .shared
            .as_ref()
            .is_some_and(|shared| shared.lock().holding_audio)
    }

    fn audio_queued(engine: &Engine) -> bool {
        engine.shared.as_ref().is_some_and(|shared| {
            let state = shared.lock();
            state
                .audio
                .as_ref()
                .is_some_and(|audio| audio.queued_bytes().is_ok_and(|bytes| bytes > 0))
        })
    }

    fn test_graphics() -> Result<RenderState> {
        let setup = gpu::setup()?;
        let instance = pollster::block_on(setup.new_instance());
        let config = WgpuConfiguration {
            wgpu_setup: setup,
            ..Default::default()
        };

        Ok(pollster::block_on(RenderState::create(
            &config,
            &instance,
            None,
            RendererOptions::default(),
        ))?)
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
            let state = engine.poll();
            ensure!(
                state.video.error.is_none(),
                "Playback failed: {:?}",
                state.video.error
            );
            if ready(engine, &state) {
                return Ok(state);
            }

            ensure!(
                Instant::now() < deadline,
                "Timed out at {:?}",
                shown(engine).ok()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn pump(
        engine: &mut Engine,
        ready: impl Fn(&Snapshot) -> bool,
    ) -> Result<Snapshot> {
        let deadline = Instant::now() + Duration::from_secs(10);

        loop {
            let state = engine.poll();
            ensure!(
                state.video.error.is_none(),
                "Playback failed: {:?}",
                state.video.error
            );
            if ready(&state) && engine.current.is_some() {
                return Ok(state);
            }

            ensure!(Instant::now() < deadline, "Decoder recovery timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
