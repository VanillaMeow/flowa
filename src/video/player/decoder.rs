//! Hardware-first candidate selection and bounded first-GOP replay with software fallback

use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use strum::{IntoStaticStr, VariantArray};

use super::{
    frame::{Frame, HardwareCodec},
    shared::{BYTE_LIMIT, PACKET_LIMIT, Shared},
    software::SoftwareDecoder,
};
use crate::{
    settings::VideoDecoder,
    video::{
        gpu::NativeGpu,
        media::{DeviceRef, Packet, Receive, Sent, Stream},
        native,
    },
};

/// Decoder backends in the order Auto tries them
#[derive(Clone, Copy, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "lowercase")]
enum Backend {
    Nvdec,
    Vulkan,
    Vaapi,
    Software,
}

impl Backend {
    fn available(self) -> bool {
        !matches!(self, Self::Vaapi) || cfg!(target_os = "linux")
    }

    fn candidates(preference: VideoDecoder) -> &'static [Self] {
        match preference {
            VideoDecoder::Auto => Self::VARIANTS,
            VideoDecoder::Nvdec => &[Self::Nvdec, Self::Software],
            VideoDecoder::Vulkan => &[Self::Vulkan, Self::Software],
            VideoDecoder::Vaapi => &[Self::Vaapi, Self::Software],
            VideoDecoder::Software => &[Self::Software],
        }
    }

    /// The shim's device type, or `None` for FFmpeg's own software decoders
    fn device(self) -> Option<native::flowa_decoder_backend> {
        match self {
            Self::Nvdec => Some(native::FLOWA_DECODER_NVDEC),
            Self::Vulkan => Some(native::FLOWA_DECODER_VULKAN),
            Self::Vaapi => Some(native::FLOWA_DECODER_VAAPI),
            Self::Software => None,
        }
    }

    fn preference(self) -> VideoDecoder {
        match self {
            Self::Nvdec => VideoDecoder::Nvdec,
            Self::Vulkan => VideoDecoder::Vulkan,
            Self::Vaapi => VideoDecoder::Vaapi,
            Self::Software => VideoDecoder::Software,
        }
    }
}

pub(super) fn available_decoders(gpu: &NativeGpu) -> Vec<VideoDecoder> {
    let mut decoders = vec![VideoDecoder::Auto];

    for &backend in Backend::VARIANTS {
        let Some(device) = backend.device().filter(|_| backend.available()) else {
            continue;
        };

        // SAFETY: The probe owns its temporary reference and borrows the live GPU
        let probe = unsafe {
            DeviceRef::from_raw(native::flowa_gpu_create_decoder_device(
                gpu.handle.as_ptr(),
                device,
            ))
        };
        if probe.is_ok() {
            decoders.push(backend.preference());
        }
    }

    decoders.push(VideoDecoder::Software);
    decoders
}

enum ActiveCodec {
    Hardware(HardwareCodec),
    Software(SoftwareDecoder),
}

impl ActiveCodec {
    fn flush(&mut self) {
        match self {
            Self::Hardware(codec) => codec.flush(),
            Self::Software(codec) => codec.flush(),
        }
    }

    fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        match self {
            Self::Hardware(codec) => codec.send(packet),
            Self::Software(codec) => codec.send(packet),
        }
    }

    fn receive(&mut self) -> Result<Receive<Frame>> {
        match self {
            Self::Hardware(codec) => codec.receive(),
            Self::Software(codec) => codec.receive(),
        }
    }
}

enum NextInput {
    Packet(Packet),
    Drain,
    Wait,
}

pub(super) struct Decoder {
    codec: Option<ActiveCodec>,
    gpu: Arc<NativeGpu>,
    stream: Arc<Stream>,
    candidate: usize,
    candidates: &'static [Backend],
    backend: Option<Backend>,
    /// The backend that produced the first frame, once probing has finished
    pub selected: Option<VideoDecoder>,
    probe: Vec<Packet>,
    probe_bytes: usize,
    replay: usize,
    probing: bool,
    unsent: Option<Packet>,
    flushed: bool,
    pub eof: bool,
}

impl Decoder {
    pub fn new(
        gpu: Arc<NativeGpu>,
        stream: Arc<Stream>,
        preference: VideoDecoder,
    ) -> Self {
        Self {
            codec: None,
            gpu,
            stream,
            candidate: 0,
            candidates: Backend::candidates(preference),
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

    pub fn can_fallback(&self) -> bool {
        matches!(
            self.backend,
            Some(Backend::Nvdec | Backend::Vulkan | Backend::Vaapi)
        )
    }

    pub fn use_software(&mut self) -> Result<()> {
        self.clear_probe();
        self.probing = true;
        self.selected = None;
        self.candidates = &[Backend::Software];
        self.candidate = 0;
        self.open_next()
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
                .context("No video codec is open")?
                .receive();
            match received {
                Ok(Receive::Item(frame)) => {
                    if self.probing {
                        self.selected = self.backend.map(Backend::preference);
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
                .codec
                .as_mut()
                .context("No video codec is open")?
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
        self.codec = None;
        self.unsent = None;

        while let Some(&backend) = self.candidates.get(self.candidate) {
            self.candidate += 1;

            if !backend.available() {
                continue;
            }

            let codec = match backend.device() {
                None => SoftwareDecoder::new(Arc::clone(&self.stream))
                    .map(ActiveCodec::Software),
                Some(device) => HardwareCodec::new(&self.gpu, &self.stream, device)
                    .map(ActiveCodec::Hardware),
            };
            match codec {
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

    fn clear_probe(&mut self) {
        self.probe.clear();
        self.probe_bytes = 0;
        self.replay = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::Backend;
    use crate::settings::VideoDecoder;
    use strum::VariantArray;

    #[test]
    fn auto_tries_hardware_backends_before_software() {
        let names: Vec<&'static str> =
            Backend::VARIANTS.iter().copied().map(Into::into).collect();

        assert_eq!(names, ["nvdec", "vulkan", "vaapi", "software"]);
    }

    #[test]
    fn an_explicit_decoder_only_falls_back_to_software() {
        for (preference, expected) in [
            (VideoDecoder::Nvdec, vec!["nvdec", "software"]),
            (VideoDecoder::Vulkan, vec!["vulkan", "software"]),
            (VideoDecoder::Vaapi, vec!["vaapi", "software"]),
            (VideoDecoder::Software, vec!["software"]),
        ] {
            let names: Vec<&'static str> = Backend::candidates(preference)
                .iter()
                .copied()
                .map(Into::into)
                .collect();
            assert_eq!(names, expected);
        }
    }
}
