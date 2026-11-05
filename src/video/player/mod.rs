//! FFmpeg playback and shared Vulkan targets owned by the UI thread

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use ash::vk;
use eframe::{
    egui_wgpu::RenderState,
    wgpu::{self, hal},
};

use super::{
    Command, VideoState,
    gpu::{self, NativeGpu},
};

mod audio;
mod clock;
mod decoder;
mod engine;
mod media;
mod rendering;
mod shared;
mod worker;

use engine::{Engine, Snapshot};

const SLOT_COUNT: usize = 3;

struct Target {
    texture: wgpu::Texture,
    id: egui::TextureId,
}

/// Owns one hardware-only player and retires native resources before wgpu targets
pub(super) struct Player {
    gpu: Arc<NativeGpu>,
    engine: Engine,
    graphics: RenderState,
    native: ash::Device,
    semaphore: vk::Semaphore,
    timeline: u64,
    targets: Vec<Target>,
    size: [u32; 2],
    slot: usize,
    displayed: Option<egui::TextureId>,
    needs_redraw: bool,
    status: Snapshot,
}

impl Player {
    pub fn new(graphics: &RenderState, ctx: &egui::Context) -> Result<Self> {
        let gpu = gpu::for_device(&graphics.device)?;
        let guard = unsafe { graphics.device.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan device unavailable")?;
        let native = guard.raw_device().clone();
        let mut timeline = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE);
        let semaphore = unsafe {
            native.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut timeline),
                None,
            )
        }?;
        drop(guard);

        Ok(Self {
            engine: Engine::new(Arc::clone(&gpu), ctx.clone()),
            gpu,
            graphics: graphics.clone(),
            native,
            semaphore,
            timeline: 0,
            targets: Vec::new(),
            size: [0; 2],
            slot: 0,
            displayed: None,
            needs_redraw: false,
            status: Snapshot::default(),
        })
    }

    pub fn load(&mut self, path: &Path) -> Result<()> {
        self.engine.open(path)?;
        self.displayed = None;
        self.status = Snapshot::default();
        self.needs_redraw = false;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.engine.stop();
        self.displayed = None;
        self.status = Snapshot::default();
        self.needs_redraw = false;
    }

    pub fn command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::TogglePause => self.engine.toggle_pause(),
            Command::Seek(seconds) => self.engine.seek(seconds),
            Command::Volume(volume) => self.engine.set_volume(volume),
        }
    }

    pub fn poll(&mut self, state: &mut VideoState) {
        if !state.active {
            return;
        }

        let status = self.engine.poll();
        if let Some(error) = &status.error {
            if state.error.as_ref() != Some(error) {
                log::error!("Video playback: {error}");
            }
            state.error = Some(error.clone());
            state.loading = false;
            self.displayed = None;
        } else {
            state.loading = status.loading;
        }

        let decoder = &status.decoder;
        if !decoder.is_empty() && state.decoder != *decoder {
            log::info!("Video decoder: {decoder} (GPU-resident frames)");
            state.decoder = decoder.clone();
        }
        let audio = &status.audio;
        if *audio != self.status.audio && !audio.is_empty() {
            log::info!("Audio output: {audio}");
        }

        state.paused = status.paused;
        state.seeking = status.seeking;
        state.position = status.position;
        state.duration = status.duration;
        state.volume = status.volume;
        state.size = status.size;
        self.needs_redraw |= status.frame_serial != self.status.frame_serial;
        self.status = status;
    }

    pub fn repaint_after(&self) -> Duration {
        let delay = self.status.next_frame_delay;
        Duration::from_secs_f64(if delay.is_finite() {
            delay.clamp(0.001, 0.1)
        } else {
            0.01
        })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.engine.stop();

        if let Some(queue) = unsafe { self.graphics.queue.as_hal::<hal::api::Vulkan>() }
        {
            queue.remove_wait_semaphore(self.semaphore);
            queue.remove_signal_semaphore(self.semaphore);
        }

        // The player is gone, so retire GPU wrappers directly before destroying its textures
        unsafe { (self.gpu.api.gpu_retire_targets)(self.gpu.handle.as_ptr()) };
        if let Err(error) = self.retire_targets_after_player() {
            log::warn!("Video target cleanup: {error:#}");
        }
        unsafe { self.native.destroy_semaphore(self.semaphore, None) };
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result, ensure};
    use eframe::{
        egui_wgpu::{RenderState, RendererOptions, WgpuConfiguration},
        wgpu,
    };

    use super::Player;
    use crate::video::Command;
    use crate::video::{VideoState, gpu};

    #[test]
    #[ignore = "Requires the FFmpeg backend, a hardware decoder, Vulkan, ffmpeg, and an audio device"]
    fn native_vulkan_playback_seek_resize_and_reload() -> Result<()> {
        let _ = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("info"),
        )
        .is_test(true)
        .try_init();
        let folder = tempfile::tempdir()?;
        let clip = folder.path().join("test.mp4");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=30",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo",
                "-t",
                "6",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ])
            .arg(&clip)
            .status()?;
        ensure!(generated.success(), "Could not generate the video fixture");

        let graphics = test_graphics()?;
        let ctx = egui::Context::default();
        let mut player = Player::new(&graphics, &ctx)?;
        let mut state = VideoState {
            active: true,
            loading: true,
            ..Default::default()
        };
        player.load(&clip)?;
        pump(&mut player, &mut state, |state| {
            state.position > 0.25 && state.size == [640, 360]
        })?;
        ensure!(
            !state.decoder.is_empty(),
            "Hardware decoding was not selected: {}",
            state.decoder
        );
        ensure!(state.duration > 5., "Video duration was not received");
        ensure!(
            player.status.audio.starts_with("sdl3/"),
            "SDL3 audio output was not selected"
        );
        let initial_frame = assert_video_pixels(&player)?;

        player.command(Command::TogglePause)?;
        pump(&mut player, &mut state, |state| state.paused)?;
        player.command(Command::Seek(3.0))?;
        pump(&mut player, &mut state, |state| {
            (state.position - 3.).abs() < 0.15
        })?;
        wait_for_changed_frame(&mut player, &mut state, initial_frame)?;
        player.render([320, 180])?;
        graphics.queue.submit([]);
        assert_video_pixels(&player)?;

        player.command(Command::Volume(25.0))?;
        pump(&mut player, &mut state, |state| {
            (state.volume - 25.).abs() < 0.01
        })?;
        player.stop();
        player.load(&clip)?;
        state.loading = true;
        pump(&mut player, &mut state, |state| {
            !state.loading && state.position < 1. && !state.paused
        })?;
        assert_video_pixels(&player)?;
        ensure!(
            (state.volume - 25.0).abs() < 0.01,
            "Volume changed on reload"
        );

        // Consecutive seeks must discard packets and audio from superseded generations
        player.command(Command::Seek(1.0))?;
        player.command(Command::Seek(4.0))?;
        player.command(Command::Seek(2.0))?;
        pump(&mut player, &mut state, |state| {
            (state.position - 2.0).abs() < 0.15
        })?;
        assert_video_pixels(&player)?;

        player.command(Command::Seek(state.duration))?;
        pump(&mut player, &mut state, |state| {
            state.paused && state.position > 5.8
        })?;
        player.command(Command::TogglePause)?;
        pump(&mut player, &mut state, |state| {
            !state.paused && state.position < 1.0
        })?;
        assert_video_pixels(&player)?;

        player.render([480, 270])?;
        drop(player);
        graphics.device.poll(wgpu::PollType::wait_indefinitely())?;
        Ok(())
    }

    fn test_graphics() -> Result<RenderState> {
        let setup = gpu::setup()?;
        let instance = pollster::block_on(setup.new_instance());
        let config = WgpuConfiguration {
            wgpu_setup: setup,
            ..Default::default()
        };
        let graphics = pollster::block_on(RenderState::create(
            &config,
            &instance,
            None,
            RendererOptions::default(),
        ))?;
        Ok(graphics)
    }

    #[test]
    #[ignore = "Requires the FFmpeg backend, Vulkan, and ffmpeg"]
    fn unsupported_hardware_never_decodes_in_software() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = folder.path().join("software-only.nut");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=128x96:rate=10",
                "-t",
                "1",
                "-c:v",
                "rawvideo",
            ])
            .arg(&clip)
            .status()?;
        ensure!(
            generated.success(),
            "Could not generate the raw-video fixture"
        );

        let graphics = test_graphics()?;
        let mut player = Player::new(&graphics, &egui::Context::default())?;
        let mut state = VideoState {
            active: true,
            loading: true,
            ..Default::default()
        };
        player.load(&clip)?;
        let deadline = Instant::now() + Duration::from_secs(5);

        while state.error.is_none() && Instant::now() < deadline {
            player.poll(&mut state);
            std::thread::sleep(Duration::from_millis(5));
        }

        ensure!(
            state.error.as_deref().is_some_and(|error| error
                .contains("Hardware decoding is unavailable")
                && error.contains("software decoding is disabled")),
            "Expected an explicit hardware-only error, got {:?}",
            state.error
        );
        ensure!(state.decoder.is_empty(), "A software decoder was selected");
        ensure!(
            player.displayed.is_none(),
            "A software frame reached the renderer"
        );
        player.stop();
        Ok(())
    }

    fn pump(
        player: &mut Player,
        state: &mut VideoState,
        ready: impl Fn(&VideoState) -> bool,
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let already_displayed = player.displayed.is_some();

        loop {
            player.poll(state);
            ensure!(state.error.is_none(), "Playback failed: {:?}", state.error);
            if already_displayed {
                ensure!(
                    !state.loading && player.displayed.is_some(),
                    "Seeking blanked the displayed frame or re-entered initial loading"
                );
            }
            if !state.loading && !state.size.contains(&0) {
                player.render(state.size)?;
            }
            player.graphics.queue.submit([]);
            player.graphics.device.poll(wgpu::PollType::Poll)?;

            if ready(state) && !state.seeking && player.displayed.is_some() {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "Playback timed out at {}s, loading={}, paused={}, size={:?}",
                state.position,
                state.loading,
                state.paused,
                state.size
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_video_pixels(player: &Player) -> Result<u64> {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let target = player
            .targets
            .iter()
            .find(|target| Some(target.id) == player.displayed)
            .context("No video texture was displayed")?;
        let row_bytes = (player.size[0] * 4).div_ceil(256) * 256;
        let buffer = player
            .graphics
            .device
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("video-test-readback"),
                size: (row_bytes * player.size[1]) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
        let mut encoder = player
            .graphics
            .device
            .create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(player.size[1]),
                },
            },
            target.texture.size(),
        );
        player.graphics.queue.submit([encoder.finish()]);

        let (sender, receiver) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        player
            .graphics
            .device
            .poll(wgpu::PollType::wait_indefinitely())?;
        receiver.recv_timeout(Duration::from_secs(5))??;

        let bytes = buffer.slice(..).get_mapped_range()?;
        let colorful = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| {
                pixel[0].abs_diff(pixel[1]) > 30 || pixel[1].abs_diff(pixel[2]) > 30
            })
            .count();
        ensure!(
            colorful > (player.size[0] * player.size[1] / 4) as usize,
            "Video texture was black or cleared, only {colorful} colorful pixels"
        );
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        Ok(hasher.finish())
    }

    fn wait_for_changed_frame(
        player: &mut Player,
        state: &mut VideoState,
        previous: u64,
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);

        loop {
            pump(player, state, |_| true)?;
            if assert_video_pixels(player)? != previous {
                return Ok(());
            }

            ensure!(
                Instant::now() < deadline,
                "Seeking did not change the rendered video frame"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
