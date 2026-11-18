//! One consistency boundary for packet generations, controls, and the audio clock

use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::Result;

use super::{clock::Clock, queues::PacketQueue};
use crate::settings::SpeedPitch;
use crate::video::{
    AudioInfo, Tracks,
    audio::{AudioFeed, AudioOutput},
    decoding::{PacketSource, SourcePacket},
    demux::InterruptFlags,
    ffmpeg::Stream,
    subtitles::{ExternalSubtitles, SubtitleCues},
};

#[derive(Clone, Copy)]
pub(super) struct Seek {
    pub target: f64,
    pub generation: u64,
}

/// Playback state shared by the UI-side engine and the demux worker
pub(super) struct PlaybackState {
    pub stream: Option<Arc<Stream>>,
    pub tracks: Tracks,
    /// The audio stream the user wants; the worker switches to it at the next seek
    pub audio_track: Option<usize>,
    /// The audio stream being decoded, which follows `audio_track`
    pub audio_info: Option<AudioInfo>,
    /// The subtitle track to show; embedded tracks switch at the next seek
    pub subtitle_track: Option<usize>,
    /// Cues of `subtitle_track`, filled by the worker or copied from a file
    pub subtitles: SubtitleCues,
    pub external_subtitles: Vec<ExternalSubtitles>,
    /// Whether to pick subtitles for a newly opened video, kept across videos
    pub subtitles_wanted: bool,
    pub packets: PacketQueue,
    pub eof: bool,
    pub seek: Option<Seek>,
    pub seeking: bool,
    pub generation: u64,
    pub flushed_generation: u64,
    pub started: bool,
    pub paused: bool,
    pub ended: bool,
    pub clock: Clock,
    /// Decoded audio the output pulls from, which outlives any one output device
    pub feed: Arc<AudioFeed>,
    pub audio: Option<AudioOutput>,
    /// Set when no output device could be opened; playback then continues silently
    pub audio_disabled: bool,
    pub pitch: SpeedPitch,
    pub volume: f64,
    pub error: Option<String>,
}

impl PlaybackState {
    pub fn new(volume: f64, rate: f64, pitch: SpeedPitch) -> Self {
        let mut clock = Clock::default();
        clock.set_rate(rate, 0.0, Instant::now());

        Self {
            stream: None,
            tracks: Tracks::default(),
            audio_track: None,
            audio_info: None,
            subtitle_track: None,
            subtitles: SubtitleCues::default(),
            external_subtitles: Vec::new(),
            subtitles_wanted: true,
            packets: PacketQueue::default(),
            eof: false,
            seek: None,
            seeking: false,
            generation: 0,
            flushed_generation: 0,
            started: false,
            paused: false,
            ended: false,
            clock,
            feed: Arc::new(AudioFeed::new(rate, pitch)),
            audio: None,
            audio_disabled: false,
            pitch,
            volume,
            error: None,
        }
    }

    /// The position of the audio being heard and the bytes SDL still holds
    pub fn heard(&self) -> Result<(Option<f64>, usize)> {
        let Some(audio) = &self.audio else {
            return Ok((None, 0));
        };
        let queued = audio.queued_bytes()?;

        Ok((self.feed.heard_position(queued, audio.latency), queued))
    }

    /// Makes the audio heard next start at `position`, dropping what SDL holds;
    /// returns false when that audio is no longer kept and must be read again
    pub fn realign_audio(&mut self, position: f64) -> Result<bool> {
        let latency = self.audio.as_ref().map_or(0., |audio| audio.latency);

        if let Some(audio) = &self.audio {
            audio.clear()?;
        }

        Ok(self.feed.move_to(position, latency))
    }

    pub fn position(&mut self, now: Instant) -> Result<f64> {
        // SDL may keep a few bytes queued, so an empty feed at the end counts as drained
        let (heard, _) = self.heard()?;
        let drained = self.eof && self.feed.ahead() <= 0.;

        Ok(self
            .clock
            .position(now, self.started && !self.paused, heard, drained))
    }

    /// Applies the speed to the clock, the feed, and SDL's resampling ratio
    pub fn set_rate(&mut self, rate: f64, now: Instant) -> Result<()> {
        let position = self.position(now)?;
        self.clock.set_rate(rate, position, now);
        self.feed.set_rate(rate);

        self.apply_ratio()
    }

    /// Switches between stretching and resampling, keeping the heard position
    pub fn set_pitch(&mut self, pitch: SpeedPitch) -> Result<()> {
        let (heard, _) = self.heard()?;
        self.pitch = pitch;
        self.feed.set_pitch(pitch);
        self.apply_ratio()?;

        if let Some(heard) = heard {
            self.realign_audio(heard)?;

            if let Some(audio) = &self.audio {
                audio.pause(self.paused || !self.started)?;
            }
        }

        Ok(())
    }

    /// SDL resamples by the rate only when pitch follows speed
    pub fn apply_ratio(&self) -> Result<()> {
        let Some(audio) = &self.audio else {
            return Ok(());
        };

        match self.pitch {
            SpeedPitch::Follow => audio.set_ratio(self.clock.rate()),
            SpeedPitch::Keep => audio.set_ratio(1.),
        }
    }
}

pub(super) struct Shared {
    state: Mutex<PlaybackState>,
    changed: Condvar,
    pub interrupt: Arc<InterruptFlags>,
    ctx: egui::Context,
}

impl Shared {
    pub fn new(ctx: egui::Context, volume: f64, rate: f64, pitch: SpeedPitch) -> Self {
        Self {
            state: Mutex::new(PlaybackState::new(volume, rate, pitch)),
            changed: Condvar::new(),
            interrupt: Arc::new(InterruptFlags::default()),
            ctx,
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, PlaybackState> {
        self.state.lock().unwrap_or_else(|poisoned| {
            let mut state = poisoned.into_inner();
            state.error.get_or_insert_with(|| {
                "The media worker failed while updating playback".into()
            });
            self.interrupt.abort.store(true, Ordering::SeqCst);
            state
        })
    }

    pub fn wait<'guard>(
        &self,
        guard: MutexGuard<'guard, PlaybackState>,
    ) -> MutexGuard<'guard, PlaybackState> {
        self.changed
            .wait_timeout(guard, Duration::from_millis(10))
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0
    }

    pub fn stopped(&self) -> bool {
        self.interrupt.abort.load(Ordering::SeqCst)
    }

    pub fn notify(&self) {
        self.changed.notify_all();
        self.ctx.request_repaint();
    }

    pub fn wake_worker(&self) {
        self.changed.notify_all();
    }

    pub fn stop(&self) {
        self.interrupt.abort.store(true, Ordering::SeqCst);
        self.changed.notify_all();
    }

    pub fn fail(&self, error: impl std::fmt::Display) {
        let mut state = self.lock();
        state.error.get_or_insert_with(|| error.to_string());
        if let Some(audio) = &state.audio {
            let _ = audio.pause(true);
        }
        self.stop();
        drop(state);
        self.notify();
    }
}

impl PacketSource for Shared {
    fn next_packet(&self) -> SourcePacket {
        let mut state = self.lock();
        if state.seeking {
            return SourcePacket::Wait;
        }

        let packet = state.packets.pop();
        let eof = state.eof;
        drop(state);
        self.wake_worker();

        match packet {
            Some(packet) => SourcePacket::Packet(packet),
            None if eof => SourcePacket::Ended,
            None => SourcePacket::Wait,
        }
    }
}
