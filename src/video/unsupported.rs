//! Stand-ins that report video playback as unavailable in builds without it
//!
//! The player type is uninhabited, so its methods can never run

const UNSUPPORTED: &str = "This build does not include video playback";

pub(crate) mod gpu {
    use anyhow::{Result, bail};
    use eframe::egui_wgpu::WgpuSetup;

    pub fn setup() -> Result<WgpuSetup> {
        bail!(super::UNSUPPORTED)
    }
}

pub(super) mod player {
    use std::{path::Path, time::Duration};

    use anyhow::{Result, bail};
    use eframe::egui_wgpu::RenderState;

    use crate::{
        settings::VideoDecoder,
        video::{PlayerCommand, VideoState},
    };

    pub(in crate::video) enum Player {}

    impl Player {
        pub fn new(_graphics: &RenderState, _ctx: &egui::Context) -> Result<Self> {
            bail!(super::UNSUPPORTED)
        }

        pub fn available_decoders(&self) -> Option<&[VideoDecoder]> {
            match *self {}
        }

        pub fn load(&mut self, _path: &Path) -> Result<()> {
            match *self {}
        }

        pub fn stop(&mut self) {
            match *self {}
        }

        pub fn command(&mut self, _command: PlayerCommand) -> Result<()> {
            match *self {}
        }

        pub fn poll(&mut self, _state: &mut VideoState) {
            match *self {}
        }

        pub fn render(&mut self, _size: [u32; 2]) -> Result<Option<egui::TextureId>> {
            match *self {}
        }

        pub fn repaint_after(&self) -> Duration {
            match *self {}
        }
    }
}

pub(super) mod thumbnail {
    use std::path::Path;

    use crate::services::worker::Generation;

    use anyhow::{Result, bail};

    pub(in crate::video) fn extract(
        _path: &Path,
        _generation: &Generation,
        _expected: u64,
    ) -> Result<egui::ColorImage> {
        bail!(super::UNSUPPORTED)
    }
}
