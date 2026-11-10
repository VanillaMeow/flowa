//! A decoded frame ready for presentation, from either kind of decoder

use std::sync::Arc;

use crate::video::{
    gpu::NativeGpu,
    media::{AvFrame, CpuFrame, FrameInfo},
};

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

    /// A frame whose images live on the rendering device, which it keeps alive
    pub fn hardware(frame: AvFrame, gpu: Arc<NativeGpu>) -> Self {
        Self::new(frame, Some(gpu))
    }

    pub fn software(frame: CpuFrame) -> Self {
        Self::new(frame.into_frame(), None)
    }

    pub fn decoded(&self) -> &AvFrame {
        &self.frame
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
