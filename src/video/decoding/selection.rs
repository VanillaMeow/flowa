//! Hardware-first decoder selection and bounded first-GOP replay with software fallback

use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};

use super::{DecodedFrame, Decoder, backend};
use crate::{
    settings::VideoDecoder,
    utils::units::MIB,
    video::{
        DecoderFallback,
        ffmpeg::{Packet, Receive, Sent, Stream},
        gpu::NativeGpu,
    },
};

/// The most packets kept for replay before the first frame, and the most packets one
/// call works through
const PROBE_PACKETS: usize = 96;
/// The most packet bytes kept for replay before the first frame
const PROBE_BYTES: usize = 32 * MIB as usize;

/// What a packet source has for the decoder
pub(in crate::video) enum SourcePacket {
    Packet(Packet),
    /// Every packet has been delivered
    Ended,
    /// No packet is ready yet
    Wait,
}

/// Where the decoder gets packets once the startup replay is used up
pub(in crate::video) trait PacketSource {
    fn next_packet(&self) -> SourcePacket;
}

/// The next input for the decoder
enum NextInput {
    Packet(Packet),
    /// Drains the decoder at the end of the stream
    Drain,
    Wait,
}

/// Decodes a stream with the first decoder that works
///
/// The first packets are kept until a frame arrives, so a decoder that fails on them
/// can be replaced by the next candidate without seeking
pub(in crate::video) struct DecoderSelection {
    decoder: Option<Box<dyn Decoder>>,
    gpu: Arc<NativeGpu>,
    stream: Arc<Stream>,
    candidate: usize,
    candidates: &'static [VideoDecoder],
    /// The decoder that is open
    active: Option<VideoDecoder>,
    /// The decoder that produced the first frame, once probing has finished
    pub selected: Option<VideoDecoder>,
    /// The decoder the user chose, or the one that failed during playback
    expected: Option<VideoDecoder>,
    /// Set when the selected decoder differs from the expected one
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
            candidates: backend::candidates(preference),
            active: None,
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

    /// Hardware decoders are always followed by at least the software decoder
    pub fn can_fallback(&self) -> bool {
        self.active.is_some_and(backend::is_hardware)
    }

    /// The decoder that is open, if any
    pub fn active(&self) -> Option<VideoDecoder> {
        self.active
    }

    /// Replaces the active decoder with the next candidate after it failed
    ///
    /// The caller seeks afterwards so the new decoder starts from a keyframe
    pub fn fall_back(&mut self) -> Result<()> {
        self.expected = self.active;
        self.clear_probe();
        self.probing = true;
        self.selected = None;
        self.open_next()
    }

    pub fn next(&mut self, source: &impl PacketSource) -> Result<Option<DecodedFrame>> {
        if self.decoder.is_none() {
            self.open_next()?;
        }

        // Bound work per UI poll even when a damaged stream repeatedly fails to decode
        for _ in 0..PROBE_PACKETS {
            let received = self
                .decoder
                .as_mut()
                .context("No video decoder is open")?
                .receive();
            match received {
                Ok(Receive::Item(frame)) => {
                    if self.probing {
                        self.selected = self.active;
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

            let packet = match self.input(source)? {
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

    fn input(&mut self, source: &impl PacketSource) -> Result<NextInput> {
        if let Some(packet) = self.unsent.take() {
            return Ok(NextInput::Packet(packet));
        }

        if self.replay < self.probe.len() {
            let packet = self.probe[self.replay].try_clone()?;
            self.replay += 1;
            return Ok(NextInput::Packet(packet));
        }

        let packet = match source.next_packet() {
            SourcePacket::Packet(packet) => packet,
            SourcePacket::Ended if !self.flushed => return Ok(NextInput::Drain),
            SourcePacket::Ended | SourcePacket::Wait => return Ok(NextInput::Wait),
        };
        if self.probing {
            ensure!(
                self.probe.len() < PROBE_PACKETS
                    && packet.bytes <= PROBE_BYTES.saturating_sub(self.probe_bytes),
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

        while let Some(&candidate) = self.candidates.get(self.candidate) {
            self.candidate += 1;

            if !backend::available(candidate) {
                continue;
            }

            match backend::open(candidate, &self.gpu, &self.stream) {
                Ok(decoder) => {
                    self.decoder = Some(decoder);
                    self.active = Some(candidate);
                    self.flushed = false;
                    self.eof = false;
                    self.replay = 0;
                    return Ok(());
                }
                Err(error) => {
                    log::debug!("{} unavailable: {error:#}", candidate.as_ref());
                }
            }
        }

        let candidates: Vec<&str> = self
            .candidates
            .iter()
            .filter(|&&candidate| backend::available(candidate))
            .map(AsRef::as_ref)
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
