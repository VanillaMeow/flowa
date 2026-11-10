//! Hardware-first decoder selection and bounded first-GOP replay with software fallback

use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};

use super::{
    decoded_frame::Frame,
    decoder::{Backend, Decoder},
    shared_state::{BYTE_LIMIT, PACKET_LIMIT, Shared},
};
use crate::{
    settings::VideoDecoder,
    video::{
        DecoderFallback,
        gpu::NativeGpu,
        media::{Packet, Receive, Sent, Stream},
    },
};

enum NextInput {
    Packet(Packet),
    Drain,
    Wait,
}

/// Decodes a stream with the first backend that works
///
/// The first packets are kept until a frame arrives, so a backend that fails on them
/// can be replaced by the next candidate without seeking
pub(super) struct DecoderSelection {
    decoder: Option<Box<dyn Decoder>>,
    gpu: Arc<NativeGpu>,
    stream: Arc<Stream>,
    candidate: usize,
    candidates: &'static [Backend],
    backend: Option<Backend>,
    /// The backend that produced the first frame, once probing has finished
    pub selected: Option<VideoDecoder>,
    /// The decoder the user chose, or the one that failed during playback
    expected: Option<VideoDecoder>,
    /// Set when the selected backend differs from the expected one
    pub fallback: Option<DecoderFallback>,
    probe: Vec<Packet>,
    probe_bytes: usize,
    replay: usize,
    probing: bool,
    unsent: Option<Packet>,
    flushed: bool,
    pub eof: bool,
}

impl DecoderSelection {
    pub fn new(
        gpu: Arc<NativeGpu>,
        stream: Arc<Stream>,
        preference: VideoDecoder,
    ) -> Self {
        Self {
            decoder: None,
            gpu,
            stream,
            candidate: 0,
            candidates: Backend::candidates(preference),
            backend: None,
            selected: None,
            // Auto quietly skips decoders this computer lacks, so only explicit choices count
            expected: match preference {
                VideoDecoder::Auto | VideoDecoder::Software => None,
                hardware => Some(hardware),
            },
            fallback: None,
            probe: Vec::new(),
            probe_bytes: 0,
            replay: 0,
            probing: true,
            unsent: None,
            flushed: false,
            eof: false,
        }
    }

    pub fn flush(&mut self) {
        if let Some(decoder) = &mut self.decoder {
            decoder.flush();
        }
        self.clear_probe();
        self.unsent = None;
        self.flushed = false;
        self.eof = false;
    }

    /// Hardware backends are always followed by at least the software decoder
    pub fn can_fallback(&self) -> bool {
        matches!(
            self.backend,
            Some(Backend::Nvdec | Backend::Vulkan | Backend::Vaapi)
        )
    }

    /// The active backend's name, for logs
    pub fn backend_name(&self) -> &'static str {
        self.backend.map_or("no decoder", Into::into)
    }

    /// Replaces the active backend with the next candidate after it failed
    ///
    /// The caller seeks afterwards so the new decoder starts from a keyframe
    pub fn fall_back(&mut self) -> Result<()> {
        self.expected = self.backend.map(Backend::preference);
        self.clear_probe();
        self.probing = true;
        self.selected = None;
        self.open_next()
    }

    pub fn next(&mut self, shared: &Shared) -> Result<Option<Frame>> {
        if self.decoder.is_none() {
            self.open_next()?;
        }

        // Bound work per UI poll even when a damaged stream repeatedly fails to decode
        for _ in 0..PACKET_LIMIT {
            let received = self
                .decoder
                .as_mut()
                .context("No video decoder is open")?
                .receive();
            match received {
                Ok(Receive::Item(frame)) => {
                    if self.probing {
                        self.selected = self.backend.map(Backend::preference);
                        self.probing = false;
                        self.clear_probe();
                        self.note_fallback();
                    }
                    return Ok(Some(frame));
                }
                Ok(Receive::End) => {
                    if self.probing && !self.probe.is_empty() {
                        self.retry()?;
                        continue;
                    }
                    ensure!(
                        !self.probing,
                        "The video stream produced no decoded frames"
                    );
                    self.eof = true;
                    return Ok(None);
                }
                Err(error) => {
                    if !self.probing {
                        return Err(error);
                    }
                    log::debug!("Video decoder failed: {error:#}");
                    self.retry()?;
                    continue;
                }
                Ok(Receive::Again) => {}
            }

            let packet = match self.input(shared)? {
                NextInput::Packet(packet) => Some(packet),
                NextInput::Drain => {
                    self.flushed = true;
                    None
                }
                NextInput::Wait => return Ok(None),
            };
            match self
                .decoder
                .as_mut()
                .context("No video decoder is open")?
                .send(packet.as_ref())
            {
                Ok(Sent::Again) => {
                    self.unsent = packet;
                    self.flushed = false;
                    return Ok(None);
                }
                Ok(Sent::Accepted | Sent::End) => {}
                Err(error) => {
                    if !self.probing {
                        return Err(error);
                    }
                    log::debug!("Video decoder rejected a packet: {error:#}");
                    self.retry()?;
                }
            }
        }
        Ok(None)
    }

    fn input(&mut self, shared: &Shared) -> Result<NextInput> {
        if let Some(packet) = self.unsent.take() {
            return Ok(NextInput::Packet(packet));
        }

        if self.replay < self.probe.len() {
            let packet = self.probe[self.replay].try_clone()?;
            self.replay += 1;
            return Ok(NextInput::Packet(packet));
        }

        let mut state = shared.lock();
        if state.seeking {
            return Ok(NextInput::Wait);
        }
        let packet = state.packets.pop();
        let eof = state.eof;
        drop(state);
        shared.wake_worker();

        let Some(packet) = packet else {
            return Ok(if eof && !self.flushed {
                NextInput::Drain
            } else {
                NextInput::Wait
            });
        };
        if self.probing {
            ensure!(
                self.probe.len() < PACKET_LIMIT
                    && packet.bytes <= BYTE_LIMIT.saturating_sub(self.probe_bytes),
                "The video decoder did not produce a frame within the probe limit"
            );
            self.probe.push(packet.try_clone()?);
            self.probe_bytes += packet.bytes;
            self.replay = self.probe.len();
        }
        Ok(NextInput::Packet(packet))
    }

    fn retry(&mut self) -> Result<()> {
        ensure!(self.probing, "The video decoder failed during playback");
        self.open_next()
    }

    fn open_next(&mut self) -> Result<()> {
        self.decoder = None;
        self.unsent = None;

        while let Some(&backend) = self.candidates.get(self.candidate) {
            self.candidate += 1;

            if !backend.available() {
                continue;
            }

            match backend.open(&self.gpu, &self.stream) {
                Ok(decoder) => {
                    self.decoder = Some(decoder);
                    self.backend = Some(backend);
                    self.flushed = false;
                    self.eof = false;
                    self.replay = 0;
                    return Ok(());
                }
                Err(error) => {
                    let name: &'static str = backend.into();
                    log::debug!("{name} unavailable: {error:#}");
                }
            }
        }

        let candidates: Vec<&'static str> = self
            .candidates
            .iter()
            .copied()
            .filter(|backend| backend.available())
            .map(Into::into)
            .collect();

        bail!(
            "Video decoding is unavailable for {}. Tried {}",
            self.stream.info.codec,
            candidates.join(", ")
        )
    }

    fn note_fallback(&mut self) {
        let (Some(from), Some(to)) = (self.expected.take(), self.selected) else {
            return;
        };

        if from != to {
            self.fallback = Some(DecoderFallback { from, to });
        }
    }

    fn clear_probe(&mut self) {
        self.probe.clear();
        self.probe_bytes = 0;
        self.replay = 0;
    }
}
