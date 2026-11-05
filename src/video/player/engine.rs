//! Rust playback lifecycle, controls, frame scheduling, and ownership of the media worker

use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
    thread::JoinHandle,
    time::Instant,
};

use anyhow::{Result, ensure};

use super::{
    audio::AudioOutput,
    decoder::Decoder,
    media::{self, Frame},
    shared::{Seek, Shared},
    worker,
};
use crate::video::{gpu::NativeGpu, native};

pub(super) struct Snapshot {
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub next_frame_delay: f64,
    pub frame_serial: u64,
    pub loading: bool,
    pub seeking: bool,
    pub paused: bool,
    pub size: [u32; 2],
    pub decoder: String,
    pub audio: String,
    pub error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            position: 0.0,
            duration: 0.0,
            volume: 100.0,
            next_frame_delay: 0.005,
            frame_serial: 0,
            loading: false,
            seeking: false,
            paused: false,
            size: [0; 2],
            decoder: String::new(),
            audio: String::new(),
            error: None,
        }
    }
}

struct TimedFrame {
    frame: Frame,
    position: f64,
}

pub(super) struct Engine {
    gpu: Arc<NativeGpu>,
    ctx: egui::Context,
    shared: Option<Arc<Shared>>,
    worker: Option<JoinHandle<()>>,
    decoder: Option<Decoder>,
    current: Option<TimedFrame>,
    pending: Option<TimedFrame>,
    applied_generation: u64,
    frame_serial: u64,
    last_pts: f64,
    volume: f64,
}

impl Engine {
    pub fn new(gpu: Arc<NativeGpu>, ctx: egui::Context) -> Self {
        Self {
            gpu,
            ctx,
            shared: None,
            worker: None,
            decoder: None,
            current: None,
            pending: None,
            applied_generation: 0,
            frame_serial: 0,
            last_pts: 0.0,
            volume: 100.0,
        }
    }

    pub fn open(&mut self, path: &Path) -> Result<()> {
        self.stop();
        let shared = Arc::new(Shared::new(self.ctx.clone(), self.volume));
        let worker = worker::spawn(
            Arc::clone(&self.gpu.api),
            path.to_owned(),
            Arc::clone(&shared),
        )?;
        self.shared = Some(shared);
        self.worker = Some(worker);
        Ok(())
    }

    pub fn stop(&mut self) {
        let Some(shared) = self.shared.take() else {
            return;
        };
        shared.stop();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            log::warn!("The media worker terminated unexpectedly");
        }

        // SAFETY: All video submissions belong to this UI thread and the demux worker has joined
        unsafe { (self.gpu.api.gpu_reset_frames)(self.gpu.handle.as_ptr()) };
        self.pending = None;
        self.current = None;
        self.decoder = None;
        self.frame_serial = 0;
        self.applied_generation = 0;
        self.last_pts = 0.0;

        // Shared owns SDL output, which must be destroyed here on the UI thread
        drop(shared);
    }

    pub fn toggle_pause(&mut self) -> Result<()> {
        let Some(shared) = &self.shared else {
            return Ok(());
        };
        let mut state = shared.lock();
        let now = Instant::now();
        let position = state.position(now)?;
        state.clock.anchor(position, now);
        state.paused = !state.paused;
        let restart = state.ended && !state.paused;
        if let Some(audio) = &state.audio {
            audio.pause(state.paused || !state.started)?;
        }
        drop(state);
        shared.notify();

        if restart {
            self.seek(0.0)?;
        }
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
        state.clock.seek(target, floor, Instant::now());
        state.packets.clear();
        shared.interrupt.seeking.store(true, Ordering::SeqCst);
        if let Some(audio) = &state.audio {
            audio.clear()?;
        }
        drop(state);

        // Keep current and its displayed texture until the requested frame replaces it
        shared.notify();
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

    pub fn poll(&mut self) -> Snapshot {
        let Some(shared) = self.shared.as_ref().map(Arc::clone) else {
            return Snapshot {
                volume: self.volume,
                ..Snapshot::default()
            };
        };

        if let Err(error) = self.prepare(&shared).and_then(|ready| {
            if ready {
                self.advance(&shared)?;
            }
            Ok(())
        }) {
            shared.fail(format!("{error:#}"));
        }

        match self.snapshot(&shared) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                shared.fail(&error);
                Snapshot {
                    error: Some(format!("{error:#}")),
                    volume: self.volume,
                    ..Snapshot::default()
                }
            }
        }
    }

    pub fn render(&mut self, target: &native::Target) -> Result<()> {
        let current = self
            .current
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No hardware frame is available"))?;
        let shared = self
            .shared
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No media is open"))?;
        let stream = shared
            .lock()
            .stream
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| anyhow::anyhow!("No video stream is available"))?;

        // SAFETY: Borrowed frames and stream parameters outlive the call; C retains GPU-flight references
        let result = native::check(|error, size| unsafe {
            (self.gpu.api.gpu_render)(
                self.gpu.handle.as_ptr(),
                current.frame.raw(),
                stream.raw(),
                target,
                error,
                size,
            )
        });
        if let Err(error) = &result {
            shared.fail(error);
        }
        ensure!(result? > 0, "The hardware frame could not be rendered");
        Ok(())
    }

    fn prepare(&mut self, shared: &Shared) -> Result<bool> {
        let mut state = shared.lock();
        if state.error.is_some() {
            return Ok(false);
        }
        let Some(stream) = state.stream.as_ref().map(Arc::clone) else {
            return Ok(false);
        };

        if stream.info.has_audio != 0 && state.audio.is_none() {
            state.audio =
                Some(AudioOutput::open(Arc::clone(&self.gpu.api), state.volume)?);
            shared.notify();
        }

        if self.decoder.is_none() {
            self.last_pts = -stream.info.frame_duration;
            self.decoder =
                Some(Decoder::new(Arc::clone(&self.gpu), Arc::clone(&stream)));
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

        for _ in 0..12 {
            if self.pending.is_none() {
                let Some(decoder) = &mut self.decoder else {
                    return Ok(());
                };
                let Some(frame) = decoder.next(shared)? else {
                    return Ok(());
                };
                let position = media::timestamp(
                    frame.info.timestamp,
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

            if pending.position + info.frame_duration * 0.5 < state.clock.floor {
                self.pending = None;
                continue;
            }
            if !starting && (state.paused || pending.position > now + 0.002) {
                return Ok(());
            }

            let position = pending.position;
            self.current = self.pending.take();
            self.frame_serial += 1;
            if starting {
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

    fn snapshot(&self, shared: &Shared) -> Result<Snapshot> {
        let mut state = shared.lock();
        let position = state.position(Instant::now())?;
        let info = state.stream.as_ref().map(|stream| stream.info);
        let duration = info.map_or(0.0, |info| info.duration);
        let end = if duration > 0.0 {
            duration
        } else {
            self.current.as_ref().map_or(0.0, |frame| {
                frame.position + info.map_or(0.0, |info| info.frame_duration)
            })
        };

        if self.decoder.as_ref().is_some_and(|decoder| decoder.eof)
            && self.pending.is_none()
            && state.started
            && position >= end
        {
            state.ended = true;
            state.paused = true;
            state.clock.anchor(end, Instant::now());
            if let Some(audio) = &state.audio {
                audio.pause(true)?;
            }
        }

        let position = if state.ended { end } else { position };
        Ok(Snapshot {
            position: if duration > 0.0 {
                position.min(duration)
            } else {
                position
            },
            duration,
            volume: state.volume,
            next_frame_delay: self
                .pending
                .as_ref()
                .map_or(0.005, |frame| (frame.position - position).max(0.0)),
            frame_serial: self.frame_serial,
            loading: self.current.is_none() && state.error.is_none(),
            seeking: self.current.is_some() && (!state.started || state.seeking),
            paused: state.paused,
            size: self.current.as_ref().map_or([0; 2], |frame| {
                frame
                    .frame
                    .display_size(info.map_or(0.0, |info| info.rotation))
            }),
            decoder: self
                .decoder
                .as_ref()
                .and_then(|decoder| decoder.selected)
                .unwrap_or_default()
                .into(),
            audio: state
                .audio
                .as_ref()
                .map_or_else(String::new, |audio| audio.name.clone()),
            error: state.error.clone(),
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}
