//! Libmpv playback and synchronized GPU targets owned by the UI thread

use std::{
    ffi::c_void,
    path::Path,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use ash::vk;
use eframe::{
    egui_wgpu::RenderState,
    wgpu::{self, hal},
};

use super::{
    VideoState,
    mpv::{self, Api},
};

mod context;
mod events;
mod rendering;

// The same ring size is declared by the private C embedding ABI
const SLOT_COUNT: usize = 3;

/// Stable callback storage shared with libmpv's decoder and render threads
struct Wakeup {
    ctx: egui::Context,
    dirty: AtomicBool,
}

unsafe extern "C" fn wakeup(data: *mut c_void) {
    // SAFETY: Callback storage outlives both registered callbacks and libmpv
    let wakeup = unsafe { &*data.cast::<Wakeup>() };
    wakeup.dirty.store(true, Ordering::Release);
    wakeup.ctx.request_repaint();
}

/// A wgpu-owned image registered with egui and borrowed by libplacebo
struct Target {
    texture: wgpu::Texture,
    id: egui::TextureId,
}

/// Owns the engine and its GPU bridge, with teardown ordered explicitly in Drop
pub(super) struct Player {
    api: Arc<Api>,
    handle: *mut c_void,
    render: *mut c_void,
    wakeup: Box<Wakeup>,
    graphics: RenderState,
    native: ash::Device,
    semaphore: vk::Semaphore,
    // Odd values release a target to mpv; even values return it to wgpu
    timeline: u64,
    targets: Vec<Target>,
    size: [u32; 2],
    slot: usize,
    displayed: Option<egui::TextureId>,
    needs_redraw: bool,
    swap_pending: bool,
}

impl Player {
    /// Borrows the application's graphics device and creates an isolated mpv client
    pub fn new(graphics: &RenderState, ctx: &egui::Context) -> Result<Self> {
        ensure!(
            graphics.adapter.get_info().backend == wgpu::Backend::Vulkan,
            "Video playback requires the Vulkan renderer"
        );
        let api = Api::load()?;

        // SAFETY: Wgpu retains ownership of these handles through graphics
        let device = unsafe { graphics.device.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan device unavailable")?;
        let native = device.raw_device().clone();
        let mut timeline = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE);
        let semaphore = unsafe {
            native.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut timeline),
                None,
            )
        }?;
        drop(device);

        let mut player = Self {
            api,
            handle: ptr::null_mut(),
            render: ptr::null_mut(),
            wakeup: Box::new(Wakeup {
                ctx: ctx.clone(),
                dirty: AtomicBool::new(true),
            }),
            graphics: graphics.clone(),
            native,
            semaphore,
            timeline: 0,
            targets: Vec::new(),
            size: [0; 2],
            slot: 0,
            displayed: None,
            needs_redraw: true,
            swap_pending: false,
        };
        player.initialize()?;

        Ok(player)
    }

    fn initialize(&mut self) -> Result<()> {
        // SAFETY: The handle is initialized and destroyed exclusively by this player
        self.handle = unsafe { (self.api.create)() };
        ensure!(!self.handle.is_null(), "Could not create libmpv");

        // Flowa owns the controls and input, while mpv owns timing and decoding
        for (name, value) in [
            (c"config", c"no"),
            (c"vo", c"libmpv"),
            // Prefer direct hardware paths and use software decoding if none works
            (c"hwdec", c"nvdec,vaapi,vulkan"),
            (c"idle", c"yes"),
            (c"keep-open", c"yes"),
            (c"input-default-bindings", c"no"),
            (c"input-vo-keyboard", c"no"),
            (c"terminal", c"no"),
            (c"video-timing-offset", c"0"),
        ] {
            self.api.option(self.handle, name, value)?;
        }

        self.api.optional_option(self.handle, c"osc", c"no")?;

        // SAFETY: Initialization precedes rendering and event processing
        self.api
            .check(unsafe { (self.api.initialize)(self.handle) })?;
        self.create_render_context()?;

        // Observed properties keep the UI independent of engine command latency
        for (name, format) in [
            (c"pause", mpv::FORMAT_FLAG),
            (c"time-pos", mpv::FORMAT_DOUBLE),
            (c"duration", mpv::FORMAT_DOUBLE),
            (c"volume", mpv::FORMAT_DOUBLE),
            (c"video-params/dw", mpv::FORMAT_INT64),
            (c"video-params/dh", mpv::FORMAT_INT64),
            (c"hwdec-current", mpv::FORMAT_STRING),
            (c"current-ao", mpv::FORMAT_STRING),
        ] {
            self.api.check(unsafe {
                (self.api.observe_property)(self.handle, 0, name.as_ptr(), format)
            })?;
        }

        let data = ptr::from_mut(self.wakeup.as_mut()).cast();

        // SAFETY: Both callbacks only request repaint and access stable callback storage
        unsafe {
            (self.api.set_wakeup_callback)(self.handle, Some(wakeup), data);
            (self.api.render_context_set_update_callback)(
                self.render,
                Some(wakeup),
                data,
            );
        }

        Ok(())
    }

    /// Replaces the current file without rebuilding the engine or GPU resources
    pub fn load(&mut self, path: &Path) -> Result<()> {
        use std::os::unix::ffi::OsStrExt;

        // Filenames use mpv's native UTF-8 interface and reject embedded NUL bytes
        let path = std::str::from_utf8(path.as_os_str().as_bytes())
            .context("Video filenames must be valid UTF-8")?;
        self.displayed = None;
        self.needs_redraw = true;
        self.command(&["loadfile", path, "replace"])?;
        self.command(&["set", "pause", "no"])
    }

    /// Queues a command without blocking the UI on decoding or I/O
    pub fn command(&self, args: &[&str]) -> Result<()> {
        self.api.command(self.handle, args)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // SAFETY: Callbacks are disabled before freeing their storage or native contexts
        unsafe {
            if !self.render.is_null() {
                (self.api.render_context_set_update_callback)(
                    self.render,
                    None,
                    ptr::null_mut(),
                );
                (self.api.render_context_free)(self.render);
            }
            if !self.handle.is_null() {
                (self.api.set_wakeup_callback)(self.handle, None, ptr::null_mut());
                (self.api.terminate_destroy)(self.handle);
            }
        }

        // A window can close before egui consumes the final staged semaphore wait
        if let Some(queue) = unsafe { self.graphics.queue.as_hal::<hal::api::Vulkan>() }
        {
            queue.remove_wait_semaphore(self.semaphore);
            queue.remove_signal_semaphore(self.semaphore);
        }

        if let Err(error) = self.clear_targets() {
            log::warn!("Video target cleanup: {error:#}");
        }

        // SAFETY: Native rendering has stopped and all uses of the semaphore have completed
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
    use crate::video::{VideoState, gpu};

    #[test]
    #[ignore = "Requires the custom libmpv backend, Vulkan GPU, ffmpeg, and an audio device"]
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

        let setup = gpu::setup()?.context("Custom video backend is missing")?;
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
        let ctx = egui::Context::default();
        let mut player = Player::new(&graphics, &ctx)?;
        player.command(&["set", "ao", "sdl3"])?;
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
            !state.decoder.is_empty() && state.decoder != "no",
            "Hardware decoding was not selected: {}",
            state.decoder
        );
        ensure!(state.duration > 5., "Video duration was not received");
        ensure!(
            player.api.audio_output(player.handle)? == "sdl3",
            "SDL3 audio output was not selected"
        );
        let initial_frame = assert_video_pixels(&player)?;

        player.command(&["set", "pause", "yes"])?;
        pump(&mut player, &mut state, |state| state.paused)?;
        player.command(&["seek", "3", "absolute+exact"])?;
        pump(&mut player, &mut state, |state| {
            (state.position - 3.).abs() < 0.15
        })?;
        wait_for_changed_frame(&mut player, &mut state, initial_frame)?;
        player.render([320, 180])?;
        graphics.queue.submit([]);
        assert_video_pixels(&player)?;

        player.command(&["set", "volume", "25"])?;
        pump(&mut player, &mut state, |state| {
            (state.volume - 25.).abs() < 0.01
        })?;
        player.command(&["stop"])?;
        player.load(&clip)?;
        state.loading = true;
        pump(&mut player, &mut state, |state| {
            !state.loading && state.position < 1. && !state.paused
        })?;
        assert_video_pixels(&player)?;

        // Software decoders can request GPU allocations before file loading completes
        player.command(&["stop"])?;
        player.command(&["set", "hwdec", "no"])?;
        player.load(&clip)?;
        state.loading = true;
        pump(&mut player, &mut state, |state| {
            state.decoder == "no" && state.position > 0.1
        })?;
        assert_video_pixels(&player)?;

        player.render([480, 270])?;
        drop(player);
        graphics.device.poll(wgpu::PollType::wait_indefinitely())?;
        Ok(())
    }

    fn pump(
        player: &mut Player,
        state: &mut VideoState,
        ready: impl Fn(&VideoState) -> bool,
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);

        loop {
            player.poll(state);
            ensure!(state.error.is_none(), "Playback failed: {:?}", state.error);
            if !state.loading && !state.size.contains(&0) {
                player.render(state.size)?;
            }
            player.graphics.queue.submit([]);
            player.graphics.device.poll(wgpu::PollType::Poll)?;

            if ready(state) && player.displayed.is_some() {
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
