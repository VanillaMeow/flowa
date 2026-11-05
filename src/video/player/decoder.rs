//! Hardware candidate selection and bounded first-GOP replay without software fallback

use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use strum::{IntoStaticStr, VariantArray};

use super::{
    media::{Frame, HardwareCodec, Packet, Receive, Sent, Stream},
    shared::{BYTE_LIMIT, PACKET_LIMIT, Shared},
};
use crate::video::{gpu::NativeGpu, native};

// Variant order matches the native hardware decoder candidate indices
#[derive(Clone, Copy, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "lowercase")]
enum Backend {
    Nvdec,
    Vulkan,
    Vaapi,
}

enum Input {
    Packet(Packet),
    Drain,
    Wait,
}

pub(super) struct Decoder {
    codec: Option<HardwareCodec>,
    gpu: Arc<NativeGpu>,
    stream: Arc<Stream>,
    candidate: usize,
    backend: Option<Backend>,
    pub selected: Option<&'static str>,
    probe: Vec<Packet>,
    probe_bytes: usize,
    replay: usize,
    probing: bool,
    unsent: Option<Packet>,
    flushed: bool,
    pub eof: bool,
}

impl Decoder {
    pub fn new(gpu: Arc<NativeGpu>, stream: Arc<Stream>) -> Self {
        Self {
            codec: None,
            gpu,
            stream,
            candidate: 0,
            backend: None,
            selected: None,
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
        if let Some(codec) = &mut self.codec {
            codec.flush();
        }
        self.clear_probe();
        self.unsent = None;
        self.flushed = false;
        self.eof = false;
    }

    pub fn next(&mut self, shared: &Shared) -> Result<Option<Frame>> {
        if self.codec.is_none() {
            self.open_next()?;
        }

        // Bound work per UI poll even when a damaged stream repeatedly fails to decode
        for _ in 0..PACKET_LIMIT {
            let received = self
                .codec
                .as_mut()
                .context("No hardware codec is open")?
                .receive();
            match received {
                Ok(Receive::Item(frame)) => {
                    if self.probing {
                        self.selected = self.backend.map(Into::into);
                        self.probing = false;
                        self.clear_probe();
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
                        "The video stream produced no hardware-decoded frames"
                    );
                    self.eof = true;
                    return Ok(None);
                }
                Err(error) => {
                    log::debug!("Hardware decoder failed: {error:#}");
                    self.retry()?;
                    continue;
                }
                Ok(Receive::Again) => {}
            }

            let packet = match self.input(shared)? {
                Input::Packet(packet) => Some(packet),
                Input::Drain => {
                    self.flushed = true;
                    None
                }
                Input::Wait => return Ok(None),
            };
            match self
                .codec
                .as_mut()
                .context("No hardware codec is open")?
                .send(packet.as_ref())
            {
                Ok(Sent::Again) => {
                    self.unsent = packet;
                    self.flushed = false;
                }
                Ok(Sent::Accepted | Sent::End) => {}
                Err(error) => {
                    log::debug!("Hardware decoder rejected a packet: {error:#}");
                    self.retry()?;
                }
            }
        }
        Ok(None)
    }

    fn input(&mut self, shared: &Shared) -> Result<Input> {
        if let Some(packet) = self.unsent.take() {
            return Ok(Input::Packet(packet));
        }

        if self.replay < self.probe.len() {
            let packet = self.probe[self.replay].try_clone()?;
            self.replay += 1;
            return Ok(Input::Packet(packet));
        }

        let mut state = shared.lock();
        if state.seeking {
            return Ok(Input::Wait);
        }
        let packet = state.packets.pop();
        let eof = state.eof;
        drop(state);
        shared.wake_worker();

        let Some(packet) = packet else {
            return Ok(if eof && !self.flushed {
                Input::Drain
            } else {
                Input::Wait
            });
        };
        if self.probing {
            ensure!(
                self.probe.len() < PACKET_LIMIT
                    && packet.bytes <= BYTE_LIMIT.saturating_sub(self.probe_bytes),
                "The hardware decoder did not produce a frame within the probe limit"
            );
            self.probe.push(packet.try_clone()?);
            self.probe_bytes += packet.bytes;
            self.replay = self.probe.len();
        }
        Ok(Input::Packet(packet))
    }

    fn retry(&mut self) -> Result<()> {
        ensure!(
            self.probing,
            "The hardware decoder failed during playback; software fallback is disabled"
        );
        self.open_next()
    }

    fn open_next(&mut self) -> Result<()> {
        self.codec = None;
        self.unsent = None;

        while let Some(&backend) = Backend::VARIANTS.get(self.candidate) {
            let index = self.candidate;
            self.candidate += 1;
            match HardwareCodec::new(&self.gpu, &self.stream, index as i32) {
                Ok(codec) => {
                    self.codec = Some(codec);
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

        bail!(
            "Hardware decoding is unavailable for {} on this GPU/driver. Tried NVDEC, Vulkan Video, and VA-API; software decoding is disabled",
            native::string(&self.stream.info.codec)
        )
    }

    fn clear_probe(&mut self) {
        self.probe.clear();
        self.probe_bytes = 0;
        self.replay = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::Backend;
    use strum::VariantArray;

    #[test]
    fn backend_order_matches_the_native_decoder_indices() {
        let names: Vec<&'static str> =
            Backend::VARIANTS.iter().copied().map(Into::into).collect();

        assert_eq!(names, ["nvdec", "vulkan", "vaapi"]);
    }
}
