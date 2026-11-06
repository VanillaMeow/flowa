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
mod software;
mod worker;

use engine::{Engine, Snapshot};

const SLOT_COUNT: usize = 3;

struct Target {
    texture: wgpu::Texture,
    id: egui::TextureId,
}

/// Owns one video player and retires native resources before wgpu targets
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
            Command::Decoder(preference) => {
                self.engine.set_decoder(preference)?;
                self.needs_redraw = true;
                Ok(())
            }
        }
    }

    pub fn available_decoders(&self) -> Vec<crate::settings::VideoDecoder> {
        decoder::available_decoders(&self.gpu)
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
            if self.status.error.is_some() {
                state.error = None;
            }
            state.loading = status.loading;
        }

        let decoder = &status.decoder;
        if !decoder.is_empty() && state.decoder != *decoder {
            if decoder == "software" {
                log::info!("Video decoder: software (CPU frames uploaded to GPU)");
            } else {
                log::info!("Video decoder: {decoder} (GPU-resident frames)");
            }
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
        state.codec.clone_from(&status.codec);
        state.audio.clone_from(&status.audio_info);
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
        io::Write,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result, ensure};
    use ash::vk;
    use eframe::{
        egui_wgpu::{RenderState, RendererOptions, WgpuConfiguration},
        wgpu::{self, hal},
    };

    use super::Player;
    use crate::settings::VideoDecoder;
    use crate::video::Command;
    use crate::video::{VideoState, gpu};

    #[test]
    #[ignore = "Requires a hardware decoder, Vulkan validation, ffmpeg, and an audio device"]
    fn native_vulkan_playback_seek_resize_and_reload() -> Result<()> {
        let flags =
            wgpu::InstanceDescriptor::new_without_display_handle_from_env().flags;
        ensure!(
            flags
                .contains(wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG),
            "Set WGPU_VALIDATION=1 and WGPU_DEBUG=1 for the hardware playback test"
        );

        let validation_errors = Arc::new(AtomicUsize::new(0));
        let reported_errors = Arc::clone(&validation_errors);
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("info"),
        )
        .filter_module("wgpu_hal::vulkan::instance", log::LevelFilter::Error)
        .format(move |buffer, record| {
            if record.level() == log::Level::Error
                && record.target() == "wgpu_hal::vulkan::instance"
            {
                reported_errors.fetch_add(1, Ordering::Relaxed);
            }

            writeln!(buffer, "{}: {}", record.level(), record.args())
        })
        .is_test(true)
        .try_init()
        .context("Could not monitor Vulkan validation errors")?;

        let folder = tempfile::tempdir()?;
        let clip = folder.path().join("test.mp4");
        let codec =
            std::env::var("FLOWA_TEST_VIDEO_CODEC").unwrap_or_else(|_| "h264".into());
        let (encoder, pixel_format) = match codec.as_str() {
            "h264" => ("libx264", "yuv420p"),
            "hevc" => ("libx265", "yuv420p10le"),
            _ => anyhow::bail!("Unsupported playback test codec: {codec}"),
        };
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
                "anullsrc=r=44100:cl=mono",
                "-t",
                "6",
                "-c:v",
                encoder,
                "-preset",
                "ultrafast",
                "-pix_fmt",
                pixel_format,
                "-c:a",
                "aac",
            ])
            .arg(&clip)
            .status()?;
        ensure!(generated.success(), "Could not generate the video fixture");

        let graphics = test_graphics()?;
        require_vulkan_validation(&graphics)?;
        let ctx = egui::Context::default();
        let mut player = Player::new(&graphics, &ctx)?;
        if let Ok(expected) = std::env::var("FLOWA_TEST_VIDEO_DECODER") {
            let preference = hardware_preference(&expected)?;
            ensure!(
                player.available_decoders().contains(&preference),
                "The selected decoder device is unavailable"
            );
            player.command(Command::Decoder(preference))?;
        }
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
            !state.decoder.is_empty() && state.decoder != "software",
            "Hardware decoding was not selected: {}",
            state.decoder
        );

        if let Ok(expected) = std::env::var("FLOWA_TEST_VIDEO_DECODER") {
            ensure!(
                state.decoder == expected,
                "Expected {expected} hardware decoding, selected {}",
                state.decoder
            );
        }

        ensure!(state.duration > 5., "Video duration was not received");
        ensure!(
            state.codec == codec,
            "Incorrect video codec: {}",
            state.codec
        );
        ensure!(
            player.status.audio.starts_with("sdl3/"),
            "SDL3 audio output was not selected"
        );
        let audio = state
            .audio
            .clone()
            .context("Audio metadata was not received")?;
        ensure!(audio.codec == "aac", "Incorrect audio codec");
        ensure!(audio.channels == 1, "Source channel count was lost");
        ensure!(audio.sample_rate == 44100, "Source sample rate was lost");
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

        let hardware_name = state.decoder.clone();
        let hardware = hardware_preference(&hardware_name)?;
        let paused_position = state.position;
        for (preference, expected) in [
            (VideoDecoder::Software, "software"),
            (hardware, hardware_name.as_str()),
        ] {
            player.command(Command::Decoder(preference))?;
            pump(&mut player, &mut state, |state| {
                state.decoder == expected && !state.seeking
            })?;
            ensure!(state.paused, "Switching decoder resumed paused playback");
            ensure!(
                (state.position - paused_position).abs() < 0.15,
                "Switching decoder lost the playback position"
            );
            ensure!(
                state.codec == codec,
                "Switching decoder lost codec metadata"
            );
            ensure!(
                state.audio.as_ref() == Some(&audio),
                "Switching lost audio metadata"
            );
            assert_video_pixels(&player)?;
        }

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
        drop(graphics);
        ensure!(
            validation_errors.load(Ordering::Relaxed) == 0,
            "Vulkan reported validation errors during hardware playback or teardown"
        );

        Ok(())
    }

    fn require_vulkan_validation(graphics: &RenderState) -> Result<()> {
        // SAFETY: The adapter and its instance stay alive throughout the query
        let adapter = unsafe { graphics.adapter.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan adapter unavailable")?;
        let instance = adapter.shared_instance();
        ensure!(
            instance.instance_api_version() >= vk::API_VERSION_1_3,
            "Vulkan 1.3 is required to query active validation tools"
        );
        ensure!(
            instance.extensions().contains(&ash::ext::debug_utils::NAME),
            "VK_EXT_debug_utils is required to monitor validation errors"
        );

        let physical_device = adapter.raw_physical_device();
        let raw_instance = instance.raw_instance();
        // SAFETY: The physical device belongs to this live instance
        let count = unsafe {
            raw_instance.get_physical_device_tool_properties_len(physical_device)
        }?;
        let mut tools = vec![vk::PhysicalDeviceToolProperties::default(); count];
        // SAFETY: Each output structure is initialized with its required sType
        unsafe {
            raw_instance
                .get_physical_device_tool_properties(physical_device, &mut tools)
        }?;
        ensure!(
            tools.iter().any(|tool| {
                tool.layer_as_c_str() == Ok(c"VK_LAYER_KHRONOS_validation")
                    && tool.purposes.contains(vk::ToolPurposeFlags::VALIDATION)
            }),
            "VK_LAYER_KHRONOS_validation is not active; install it or set VK_LAYER_PATH"
        );

        Ok(())
    }

    fn hardware_preference(name: &str) -> Result<VideoDecoder> {
        match name {
            "nvdec" => Ok(VideoDecoder::Nvdec),
            "vulkan" => Ok(VideoDecoder::Vulkan),
            "vaapi" => Ok(VideoDecoder::Vaapi),
            _ => anyhow::bail!("Unsupported hardware decoder: {name}"),
        }
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
    fn unsupported_hardware_falls_back_to_software() -> Result<()> {
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
                "3",
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
        pump(&mut player, &mut state, |state| state.position > 0.1)?;
        ensure!(
            state.decoder == "software",
            "Software fallback was not selected"
        );
        let initial_frame = assert_video_pixels(&player)?;

        player.command(Command::TogglePause)?;
        pump(&mut player, &mut state, |state| state.paused)?;
        player.command(Command::Seek(2.0))?;
        player.command(Command::Seek(1.5))?;
        pump(&mut player, &mut state, |state| {
            (state.position - 1.5).abs() < 0.15
        })?;
        wait_for_changed_frame(&mut player, &mut state, initial_frame)?;
        player.render([256, 192])?;
        graphics.queue.submit([]);
        assert_video_pixels(&player)?;

        player.command(Command::Seek(state.duration))?;
        pump(&mut player, &mut state, |state| state.position > 2.8)?;
        player.command(Command::TogglePause)?;
        pump(&mut player, &mut state, |state| {
            state.paused && state.position > 2.8
        })?;
        player.command(Command::TogglePause)?;
        pump(&mut player, &mut state, |state| {
            !state.paused && state.position < 1.0
        })?;

        // Closing while the CPU worker has queued frames must not block teardown
        player.stop();
        player.load(&clip)?;
        state.loading = true;
        pump(&mut player, &mut state, |state| {
            !state.loading && state.position < 1.0
        })?;
        assert_video_pixels(&player)?;
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
