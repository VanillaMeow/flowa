//! One consistency boundary for packet generations, controls, and the SDL audio clock

use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, MutexGuard, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};

use super::{
    audio_output::{AudioOutput, BYTES_PER_SECOND},
    clock::{AudioTiming, Clock},
};
use crate::{
    utils::units::MIB,
    video::media::{InterruptFlags, Packet, Stream},
};

pub(super) const PACKET_LIMIT: usize = 96;
pub(super) const BYTE_LIMIT: usize = 32 * MIB as usize;

#[derive(Default)]
pub(super) struct PacketQueue {
    packets: VecDeque<Packet>,
    bytes: usize,
}

impl PacketQueue {
    pub fn has_capacity(&self) -> bool {
        self.packets.len() < PACKET_LIMIT && self.bytes < BYTE_LIMIT
    }

    pub fn can_push(&self, bytes: usize) -> bool {
        self.packets.len() < PACKET_LIMIT
            && bytes <= BYTE_LIMIT.saturating_sub(self.bytes)
    }

    pub fn push(&mut self, packet: Packet) -> Result<()> {
        ensure!(
            self.can_push(packet.bytes),
            "Video packet buffer limit exceeded"
        );
        self.bytes += packet.bytes;
        self.packets.push_back(packet);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<Packet> {
        let packet = self.packets.pop_front()?;
        self.bytes -= packet.bytes;
        Some(packet)
    }

    pub fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
    }
}

#[derive(Clone, Copy)]
pub(super) struct Seek {
    pub target: f64,
    pub generation: u64,
}

/// Playback state shared by the UI-side engine and the demux worker
pub(super) struct PlaybackState {
    pub stream: Option<Arc<Stream>>,
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
    pub audio: Option<AudioOutput>,
    /// Set when no output device could be opened; playback then continues silently
    pub audio_disabled: bool,
    pub volume: f64,
    pub error: Option<String>,
}

impl PlaybackState {
    pub fn new(volume: f64) -> Self {
        Self {
            stream: None,
            packets: PacketQueue::default(),
            eof: false,
            seek: None,
            seeking: false,
            generation: 0,
            flushed_generation: 0,
            started: false,
            paused: false,
            ended: false,
            clock: Clock::default(),
            audio: None,
            audio_disabled: false,
            volume,
            error: None,
        }
    }

    pub fn position(&mut self, now: Instant) -> Result<f64> {
        let timing = self
            .audio
            .as_ref()
            .map(|audio| {
                Ok::<_, anyhow::Error>(AudioTiming {
                    queued: audio.queued_bytes()? as f64 / BYTES_PER_SECOND as f64,
                    latency: audio.latency,
                })
            })
            .transpose()?;
        Ok(self
            .clock
            .position(now, self.started && !self.paused, timing, self.eof))
    }
}

pub(super) struct Shared {
    state: Mutex<PlaybackState>,
    changed: Condvar,
    pub interrupt: Arc<InterruptFlags>,
    ctx: egui::Context,
}

impl Shared {
    pub fn new(ctx: egui::Context, volume: f64) -> Self {
        Self {
            state: Mutex::new(PlaybackState::new(volume)),
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
