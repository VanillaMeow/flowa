//! Playback frames and the hardware codecs tied to the rendering device

use std::sync::Arc;

use anyhow::Result;

use crate::video::{
    ffmpeg,
    gpu::NativeGpu,
    media::{
        AvFrame, Codec, CpuFrame, DeviceRef, FrameInfo, Packet, Receive, Sent, Stream,
    },
    native,
};

/// Keeps the rendering device alive until the codec and its borrowed device references are released
pub(super) struct HardwareCodec {
    codec: Codec,
    gpu: Arc<NativeGpu>,
}

impl HardwareCodec {
    pub fn new(
        gpu: &Arc<NativeGpu>,
        stream: &Stream,
        backend: native::flowa_decoder_backend,
    ) -> Result<Self> {
        // SAFETY: The GPU transfers one device reference, and this owner retains the borrowed GPU
        let device = unsafe {
            DeviceRef::from_raw(native::flowa_gpu_create_decoder_device(
                gpu.handle.as_ptr(),
                backend,
            ))
        }?;
        let codec = Codec::hardware(stream, device)?;

        Ok(Self {
            codec,
            gpu: Arc::clone(gpu),
        })
    }

    pub fn flush(&mut self) {
        self.codec.flush();
    }

    pub fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        self.codec.send(packet)
    }

    pub fn receive(&mut self) -> Result<Receive<Frame>> {
        match self.codec.receive()? {
            Receive::Again => Ok(Receive::Again),
            Receive::End => Ok(Receive::End),
            Receive::Item(frame) => Ok(Receive::Item(Frame::new(
                frame,
                Some(Arc::clone(&self.gpu)),
            ))),
        }
    }
}

pub(super) struct Frame {
    frame: AvFrame,
    pub info: FrameInfo,
    // Hardware frames borrow the device; CPU frames have no rendering-device dependency
    _gpu: Option<Arc<NativeGpu>>,
}

impl Frame {
    fn new(frame: AvFrame, gpu: Option<Arc<NativeGpu>>) -> Self {
        let info = frame.info();

        Self {
            frame,
            info,
            _gpu: gpu,
        }
    }

    pub fn software(frame: CpuFrame) -> Self {
        Self::new(frame.into_frame(), None)
    }

    pub fn raw(&self) -> *mut ffmpeg::AVFrame {
        self.frame.raw()
    }

    pub fn display_size(&self, stream_rotation: f64) -> [u32; 2] {
        let aspect = if self.info.aspect_num > 0 && self.info.aspect_den > 0 {
            f64::from(self.info.aspect_num) / f64::from(self.info.aspect_den)
        } else {
            1.0
        };
        let mut size = [
            (f64::from(self.info.width.max(0)) * aspect).round() as u32,
            self.info.height.max(0) as u32,
        ];
        let rotation = if self.info.rotation.is_finite() {
            self.info.rotation
        } else {
            stream_rotation
        };
        let quarter_turns = (rotation.abs() / 90.0).round() as u32;

        if rotation.is_finite() && !quarter_turns.is_multiple_of(2) {
            size.swap(0, 1);
        }

        size
    }
}
