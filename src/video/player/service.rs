//! Keeps driver calls and playback ownership off the UI thread

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, atomic::Ordering, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Result, ensure};
use eframe::egui_wgpu::RenderState;

use super::{Pipeline, Snapshot, Target, audio::AudioRuntime, decoder::DecoderProbe};
use crate::{
    settings::VideoDecoder,
    video::{
        PlayerCommand, VideoState,
        gpu::{self, NativeGpu},
    },
};

#[derive(Default)]
struct Requests {
    /// A new file (or `None` to stop), tagged with the UI's epoch
    source: Option<(u64, Option<PathBuf>)>,
    commands: Vec<PlayerCommand>,
    /// A requested playback step, optionally with the target texture size
    tick: Option<Option<[u32; 2]>>,
    shutdown: bool,
}

#[derive(Default)]
struct Mailbox {
    requests: Mutex<Requests>,
    wake: Condvar,
}

impl Mailbox {
    fn update(&self, update: impl FnOnce(&mut Requests)) {
        let mut requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        update(&mut requests);
        self.wake.notify_one();
    }

    fn take(&self) -> Requests {
        let mut requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());

        while requests.source.is_none()
            && requests.commands.is_empty()
            && requests.tick.is_none()
            && !requests.shutdown
        {
            requests = self.wake.wait(requests).unwrap_or_else(|e| e.into_inner());
        }

        std::mem::take(&mut *requests)
    }
}

impl Requests {
    fn replace_source(&mut self, epoch: u64, path: Option<PathBuf>) {
        self.source = Some((epoch, path));
        self.tick = self.tick.map(|_| None);
        self.commands.retain(|command| {
            matches!(
                command,
                PlayerCommand::Volume(_) | PlayerCommand::Decoder(_)
            )
        });
    }

    /// Keeps only the latest command of each kind; two queued toggles cancel out
    fn command(&mut self, command: PlayerCommand) {
        let previous = self.commands.iter().position(|queued| {
            std::mem::discriminant(queued) == std::mem::discriminant(&command)
        });
        if let Some(index) = previous {
            self.commands.remove(index);
            if matches!(command, PlayerCommand::TogglePause) {
                return;
            }
        }
        self.commands.push(command);
    }
}

struct Completed {
    epoch: u64,
    status: Snapshot,
    target: Option<Target>,
    retired: Vec<Target>,
}

struct Worker {
    mailbox: Arc<Mailbox>,
    results: mpsc::Receiver<Completed>,
    thread: Option<JoinHandle<Vec<Target>>>,
    pending: bool,
}

impl Worker {
    fn request_tick(&mut self, size: Option<[u32; 2]>) {
        if self.pending {
            return;
        }

        self.pending = true;
        self.mailbox.update(|requests| requests.tick = Some(size));
    }
}

struct PlaybackClaim(Arc<NativeGpu>);

impl PlaybackClaim {
    fn new(gpu: Arc<NativeGpu>) -> Result<Self> {
        ensure!(
            gpu.playback_claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "The video device already has a playback owner"
        );
        Ok(Self(gpu))
    }
}

impl Drop for PlaybackClaim {
    fn drop(&mut self) {
        self.0.playback_claimed.store(false, Ordering::Release);
    }
}

enum Driver {
    Background(Worker),
    Inline(Box<Pipeline>),
}

/// The UI reads completed frames without waiting for decoding or shader compilation
pub(in crate::video) struct Player {
    driver: Driver,
    graphics: RenderState,
    ctx: egui::Context,
    epoch: u64,
    requested_size: Option<[u32; 2]>,
    status: Snapshot,
    displayed: Option<egui::TextureId>,
    target: Option<Target>,
    decoders: DecoderProbe,
    audio_name: String,
    // Dropped on the UI thread after playback has joined and destroyed its SDL streams
    _audio: Option<AudioRuntime>,
    _claim: PlaybackClaim,
}

impl Player {
    pub fn new(graphics: &RenderState, ctx: &egui::Context) -> Result<Self> {
        let gpu = gpu::for_device(&graphics.device)?;
        let claim = PlaybackClaim::new(Arc::clone(&gpu))?;
        let audio = AudioRuntime::new()
            .inspect_err(|error| {
                log::warn!("Audio initialization: {error:#}");
            })
            .ok();
        let decoders = DecoderProbe::new(Arc::clone(&gpu), ctx.clone());
        let driver = if gpu.background_playback {
            let mailbox = Arc::new(Mailbox::default());
            let requests = Arc::clone(&mailbox);
            let (sender, results) = mpsc::sync_channel(1);
            let graphics = graphics.clone();
            let ctx = ctx.clone();
            let thread = thread::Builder::new()
                .name("video-playback".into())
                .spawn(move || run(gpu, graphics, ctx, requests, sender))?;
            Driver::Background(Worker {
                mailbox,
                results,
                thread: Some(thread),
                pending: false,
            })
        } else {
            // A shared Vulkan queue requires all native submissions to stay on the UI thread
            log::info!(
                "Video playback uses the UI thread: no separate graphics queue available"
            );
            Driver::Inline(Box::new(Pipeline::new(graphics, ctx)?))
        };

        Ok(Self {
            driver,
            graphics: graphics.clone(),
            ctx: ctx.clone(),
            epoch: 0,
            requested_size: None,
            status: Snapshot::default(),
            displayed: None,
            target: None,
            decoders,
            audio_name: String::new(),
            _audio: audio,
            _claim: claim,
        })
    }

    pub fn load(&mut self, path: &Path) -> Result<()> {
        self.replace_source(Some(path))
    }

    pub fn stop(&mut self) {
        if let Err(error) = self.replace_source(None) {
            log::warn!("Video stop: {error:#}");
        }
    }

    fn replace_source(&mut self, path: Option<&Path>) -> Result<()> {
        self.epoch += 1;
        self.displayed = None;
        self.target = None;
        self.requested_size = None;
        self.status = Snapshot {
            video: VideoState {
                loading: path.is_some(),
                volume: self.status.video.volume,
                ..VideoState::default()
            },
            ..Snapshot::default()
        };
        match &mut self.driver {
            Driver::Background(worker) => worker.mailbox.update(|requests| {
                requests.replace_source(self.epoch, path.map(Path::to_owned));
            }),
            Driver::Inline(playback) => {
                if let Some(path) = path {
                    playback.load(path)?;
                } else {
                    playback.stop();
                }
            }
        }
        Ok(())
    }

    pub fn command(&mut self, command: PlayerCommand) -> Result<()> {
        match &mut self.driver {
            Driver::Background(worker) => {
                worker.mailbox.update(|requests| requests.command(command))
            }
            Driver::Inline(playback) => playback.command(command)?,
        }
        Ok(())
    }

    /// The decoders this device supports, or `None` while they are being detected
    pub fn available_decoders(&self) -> Option<&[VideoDecoder]> {
        self.decoders.decoders()
    }

    fn receive(&mut self) {
        let completed = match &mut self.driver {
            Driver::Background(worker) => match worker.results.try_recv() {
                Ok(completed) => {
                    worker.pending = false;
                    Some(completed)
                }
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status.video.error =
                        Some("The video playback worker stopped unexpectedly".into());
                    None
                }
            },
            Driver::Inline(playback) => {
                playback.poll();
                Some(completion(playback, self.epoch))
            }
        };
        let Some(completed) = completed else { return };
        if completed.epoch == self.epoch {
            self.status = completed.status;
            self.displayed = completed.target.as_ref().map(|target| target.id);
            self.target = completed.target;
        }
        self.free_targets(completed.retired);
    }

    fn request_tick(&mut self) {
        if let Driver::Background(worker) = &mut self.driver {
            worker.request_tick(self.requested_size);
        }
    }

    pub fn poll(&mut self, state: &mut VideoState) {
        self.receive();

        if !state.active {
            return;
        }

        self.request_tick();
        self.apply_state(state);
    }

    pub fn render(&mut self, size: [u32; 2]) -> Result<Option<egui::TextureId>> {
        self.requested_size = Some(size);
        if let Driver::Inline(playback) = &mut self.driver {
            playback.render(size)?;
            let completed = completion(playback, self.epoch);
            self.displayed = completed.target.as_ref().map(|target| target.id);
            self.target = completed.target;
            self.free_targets(completed.retired);
        } else if self.target.as_ref().is_none_or(|target| {
            [target.texture.width(), target.texture.height()] != size
        }) {
            // A paused video also needs another UI poll while its replacement target is prepared
            self.ctx.request_repaint_after(Duration::from_millis(10));
        }
        Ok(self.displayed)
    }

    fn apply_state(&mut self, state: &mut VideoState) {
        if !state.active {
            return;
        }

        self.log_changes(state);

        let video = &self.status.video;
        *state = VideoState {
            active: true,
            loading: video.loading && video.error.is_none(),
            ..video.clone()
        };

        if state.error.is_some() {
            self.displayed = None;
        }
    }

    fn log_changes(&mut self, previous: &VideoState) {
        let video = &self.status.video;

        if let Some(error) = &video.error
            && previous.error.as_ref() != Some(error)
        {
            log::error!("Video playback: {error}");
        }

        match video.decoder {
            Some(decoder) if previous.decoder != video.decoder => {
                let frames = if decoder == VideoDecoder::Software {
                    "CPU frames uploaded to GPU"
                } else {
                    "GPU-resident frames"
                };
                log::info!("Video decoder: {} ({frames})", decoder.as_ref());
            }
            _ => {}
        }

        let output = &self.status.audio_output;
        if !output.is_empty() && *output != self.audio_name {
            log::info!("Audio output: {output}");
        }
        self.audio_name.clone_from(output);
    }

    pub fn repaint_after(&self) -> Duration {
        delay(&self.status)
    }

    fn free_targets(&self, targets: Vec<Target>) {
        if targets.is_empty() {
            return;
        }

        let mut renderer = self.graphics.renderer.write();

        for target in targets {
            renderer.free_texture(&target.id);
        }
    }
}

/// How long the UI may sleep before the next frame is due, kept within 1–100 ms
fn delay(status: &Snapshot) -> Duration {
    let seconds = status.next_frame_delay;
    Duration::from_secs_f64(if seconds.is_finite() {
        seconds.clamp(0.001, 0.1)
    } else {
        0.01
    })
}

fn completion(playback: &mut Pipeline, epoch: u64) -> Completed {
    Completed {
        epoch,
        status: playback.status.clone(),
        target: playback
            .targets
            .iter()
            .find(|target| Some(target.id) == playback.displayed)
            .cloned(),
        retired: std::mem::take(&mut playback.retired),
    }
}

fn run(
    _gpu: Arc<NativeGpu>,
    graphics: RenderState,
    ctx: egui::Context,
    requests: Arc<Mailbox>,
    results: mpsc::SyncSender<Completed>,
) -> Vec<Target> {
    // Construct the engine here so hardware frame pointers never cross threads
    let mut playback = match Pipeline::new(&graphics, &ctx) {
        Ok(playback) => playback,
        Err(error) => {
            log::error!("Could not start video playback: {error:#}");
            ctx.request_repaint();
            return Vec::new();
        }
    };
    let mut epoch = 0;
    let mut failure = None;

    loop {
        let work = requests.take();
        if work.shutdown {
            break;
        }

        if let Some((next_epoch, path)) = work.source {
            epoch = next_epoch;
            failure = if let Some(path) = path {
                playback.load(&path).err()
            } else {
                playback.stop();
                None
            };
        }

        let changed = !work.commands.is_empty();

        for command in work.commands {
            if matches!(command, PlayerCommand::Decoder(_)) {
                failure = None;
            }
            if let Err(error) = playback.command(command) {
                failure = Some(error);
            }
        }

        let Some(size) = work.tick else {
            if changed {
                ctx.request_repaint();
            }
            continue;
        };

        let previous_serial = playback.status.frame_serial;
        let previous_displayed = playback.displayed;
        let previous_error = playback.status.video.error.clone();
        playback.poll();
        if failure.is_none()
            && playback.status.video.error.is_none()
            && playback.status.frame_serial > 0
        {
            let size = size.unwrap_or(playback.status.video.size);
            if let Err(error) = playback.render(size) {
                failure = Some(error);
            }
        }

        let video = &mut playback.status.video;
        if let Some(error) = &failure {
            video.error = Some(format!("{error:#}"));
        }

        let notify = changed
            || previous_serial != playback.status.frame_serial
            || previous_displayed != playback.displayed
            || previous_error != video.error;
        let active =
            video.error.is_none() && (video.loading || video.seeking || !video.paused);
        let next = delay(&playback.status);

        if results.send(completion(&mut playback, epoch)).is_err() {
            break;
        }

        if notify {
            ctx.request_repaint();
        } else if active {
            ctx.request_repaint_after(next);
        }
    }

    playback.engine.stop();

    // Native wrappers are retired by Playback::drop before the join releases these textures
    let mut retired = std::mem::take(&mut playback.retired);
    retired.append(&mut playback.targets);
    retired
}

impl Drop for Player {
    fn drop(&mut self) {
        let mut retired = Vec::new();
        match &mut self.driver {
            Driver::Background(worker) => {
                worker.mailbox.update(|requests| requests.shutdown = true);
                if let Some(thread) = worker.thread.take() {
                    match thread.join() {
                        Ok(targets) => retired.extend(targets),
                        Err(_) => log::warn!(
                            "The video playback worker terminated unexpectedly"
                        ),
                    }
                }
                while let Ok(completed) = worker.results.try_recv() {
                    retired.extend(completed.retired);
                }
            }
            Driver::Inline(playback) => {
                playback.stop();
                if let Err(error) = playback.clear_targets() {
                    log::warn!("Video target cleanup: {error:#}");
                }
                retired.append(&mut playback.retired);
                retired.append(&mut playback.targets);
            }
        }
        self.target = None;
        self.free_targets(retired);
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

    use super::{Completed, Driver, Mailbox, Player, Requests, Snapshot, Worker};
    use crate::settings::VideoDecoder;
    use crate::video::{PlayerCommand, VideoState, gpu};

    #[test]
    fn a_stalled_worker_keeps_ui_requests_bounded_and_nonblocking() -> Result<()> {
        let mailbox = Arc::new(Mailbox::default());
        let requests = Arc::clone(&mailbox);
        let (results, receiver) = mpsc::sync_channel(1);
        let (started, ready) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            assert!(requests.take().tick.is_some());
            started
                .send(())
                .expect("The UI should still own its startup receiver");
            gate.recv_timeout(Duration::from_secs(5))
                .expect("UI requests blocked behind the stalled worker");
            results
                .send(Completed {
                    epoch: 1,
                    status: Snapshot::default(),
                    target: None,
                    retired: Vec::new(),
                })
                .expect("The UI should still own its result receiver");
            Vec::new()
        });
        let mut worker = Worker {
            mailbox,
            results: receiver,
            thread: Some(thread),
            pending: false,
        };
        worker.request_tick(None);
        ready.recv_timeout(Duration::from_secs(5))?;

        // The worker cannot finish until after these UI operations have returned
        for index in 0..1000 {
            worker.mailbox.update(|requests| {
                requests.replace_source(index, Some("latest.mp4".into()));
                requests.command(PlayerCommand::Seek(index as f64));
                requests.command(PlayerCommand::Volume(25.0));
                requests.command(PlayerCommand::Decoder(VideoDecoder::Vulkan));
            });
            worker.request_tick(Some([320, 180]));
            assert!(matches!(
                worker.results.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
        }
        let queued = worker.mailbox.take();
        assert!(matches!(queued.source, Some((999, _))));
        assert_eq!(queued.commands.len(), 3);
        assert!(
            queued.tick.is_none(),
            "More than one playback tick was queued"
        );

        release.send(())?;
        assert_eq!(
            worker.results.recv_timeout(Duration::from_secs(5))?.epoch,
            1
        );
        assert!(
            worker
                .thread
                .take()
                .expect("The worker should be running")
                .join()
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn replacing_a_file_discards_old_controls_and_preserves_preferences() {
        let mut requests = Requests::default();
        requests.command(PlayerCommand::TogglePause);
        requests.command(PlayerCommand::Seek(12.0));
        requests.command(PlayerCommand::Volume(30.0));
        requests.command(PlayerCommand::Decoder(VideoDecoder::Nvdec));
        requests.replace_source(2, Some("other.mp4".into()));

        assert_eq!(requests.commands.len(), 2);
        assert!(matches!(requests.commands[0], PlayerCommand::Volume(30.0)));
        assert!(matches!(
            requests.commands[1],
            PlayerCommand::Decoder(VideoDecoder::Nvdec)
        ));
        requests.command(PlayerCommand::TogglePause);
        requests.command(PlayerCommand::TogglePause);
        assert_eq!(requests.commands.len(), 2);
    }

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
        ensure!(
            matches!(player.driver, Driver::Background(_)),
            "This test requires a separate UI graphics queue"
        );
        if let Ok(expected) = std::env::var("FLOWA_TEST_VIDEO_DECODER") {
            let preference = hardware_preference(&expected)?;
            ensure!(
                wait_for_decoders(&player)?.contains(&preference),
                "The selected decoder device is unavailable"
            );
            player.command(PlayerCommand::Decoder(preference))?;
        }
        let mut state = VideoState {
            active: true,
            loading: true,
            ..Default::default()
        };
        player.load(&folder.path().join("missing.mp4"))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.error.is_none() {
            player.poll(&mut state);
            ensure!(
                Instant::now() < deadline,
                "The failed load did not report its error"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        player.load(&clip)?;
        pump(&mut player, &mut state, |state| {
            state.position > 0.25 && state.size == [640, 360]
        })?;
        let hardware = state
            .decoder
            .filter(|decoder| *decoder != VideoDecoder::Software)
            .with_context(|| {
                format!("Hardware decoding was not selected: {:?}", state.decoder)
            })?;

        if let Ok(expected) = std::env::var("FLOWA_TEST_VIDEO_DECODER") {
            ensure!(
                hardware == hardware_preference(&expected)?,
                "Expected {expected} hardware decoding, selected {hardware:?}"
            );
        }

        ensure!(state.duration > 5., "Video duration was not received");
        ensure!(
            state.codec == codec,
            "Incorrect video codec: {}",
            state.codec
        );
        ensure!(
            player.status.audio_output.starts_with("sdl3/"),
            "SDL3 audio output was not selected"
        );
        let audio = state.audio.context("Audio metadata was not received")?;
        ensure!(audio.codec == "aac", "Incorrect audio codec");
        ensure!(audio.channels == 1, "Source channel count was lost");
        ensure!(audio.sample_rate == 44100, "Source sample rate was lost");
        let initial_frame = assert_video_pixels(&player)?;

        player.command(PlayerCommand::TogglePause)?;
        pump(&mut player, &mut state, |state| state.paused)?;
        player.command(PlayerCommand::Seek(3.0))?;
        pump(&mut player, &mut state, |state| {
            (state.position - 3.).abs() < 0.15
        })?;
        wait_for_changed_frame(&mut player, &mut state, initial_frame)?;
        resize(&mut player, &mut state, [320, 180])?;
        graphics.queue.submit([]);
        assert_video_pixels(&player)?;

        let paused_position = state.position;
        for preference in [VideoDecoder::Software, hardware] {
            player.command(PlayerCommand::Decoder(preference))?;
            pump(&mut player, &mut state, |state| {
                state.decoder == Some(preference) && !state.seeking
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

        player.command(PlayerCommand::Volume(25.0))?;
        pump(&mut player, &mut state, |state| {
            (state.volume - 25.).abs() < 0.01
        })?;
        player.stop();
        // Replace a pending load and discard any snapshot from the previous file
        player.load(&folder.path().join("superseded.mp4"))?;
        player.poll(&mut state);
        player.load(&clip)?;
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
        player.command(PlayerCommand::Seek(1.0))?;
        player.command(PlayerCommand::Seek(4.0))?;
        player.command(PlayerCommand::Seek(2.0))?;
        pump(&mut player, &mut state, |state| {
            (state.position - 2.0).abs() < 0.15
        })?;
        assert_video_pixels(&player)?;

        player.command(PlayerCommand::Seek(state.duration))?;
        pump(&mut player, &mut state, |state| {
            state.paused && state.position > 5.8
        })?;
        player.command(PlayerCommand::TogglePause)?;
        pump(&mut player, &mut state, |state| {
            !state.paused && state.position < 1.0
        })?;
        assert_video_pixels(&player)?;

        resize(&mut player, &mut state, [480, 270])?;
        drop(player);
        graphics.device.poll(wgpu::PollType::wait_indefinitely())?;
        drop(graphics);
        ensure!(
            validation_errors.load(Ordering::Relaxed) == 0,
            "Vulkan reported validation errors during hardware playback or teardown"
        );

        Ok(())
    }

    fn wait_for_decoders(player: &Player) -> Result<Vec<VideoDecoder>> {
        let deadline = Instant::now() + Duration::from_secs(10);

        loop {
            if let Some(decoders) = player.available_decoders() {
                return Ok(decoders.to_vec());
            }

            ensure!(Instant::now() < deadline, "Decoder detection timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
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
        ensure!(
            matches!(player.driver, Driver::Background(_)),
            "This test requires a separate UI graphics queue"
        );
        let mut state = VideoState {
            active: true,
            loading: true,
            ..Default::default()
        };
        player.load(&clip)?;
        pump(&mut player, &mut state, |state| state.position > 0.1)?;
        ensure!(
            state.decoder == Some(VideoDecoder::Software),
            "Software fallback was not selected"
        );
        let initial_frame = assert_video_pixels(&player)?;

        player.command(PlayerCommand::TogglePause)?;
        pump(&mut player, &mut state, |state| state.paused)?;
        player.command(PlayerCommand::Seek(2.0))?;
        player.command(PlayerCommand::Seek(1.5))?;
        pump(&mut player, &mut state, |state| {
            (state.position - 1.5).abs() < 0.15
        })?;
        wait_for_changed_frame(&mut player, &mut state, initial_frame)?;
        resize(&mut player, &mut state, [256, 192])?;
        graphics.queue.submit([]);
        assert_video_pixels(&player)?;

        player.command(PlayerCommand::Seek(state.duration))?;
        pump(&mut player, &mut state, |state| state.position > 2.8)?;
        player.command(PlayerCommand::TogglePause)?;
        pump(&mut player, &mut state, |state| {
            state.paused && (state.position - state.duration).abs() < 0.01
        })?;
        player.command(PlayerCommand::TogglePause)?;
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
        let mut maximum_ui_time = Duration::ZERO;
        let mut maximum_update_time = Duration::ZERO;

        loop {
            let poll_started = Instant::now();
            player.poll(state);
            maximum_ui_time = maximum_ui_time.max(poll_started.elapsed());
            ensure!(state.error.is_none(), "Playback failed: {:?}", state.error);
            if already_displayed {
                ensure!(
                    !state.loading && player.displayed.is_some(),
                    "Seeking blanked the displayed frame or re-entered initial loading"
                );
            }
            if !state.loading && !state.size.contains(&0) {
                let render_started = Instant::now();
                player.render(state.size)?;
                maximum_ui_time = maximum_ui_time.max(render_started.elapsed());
            }
            player.graphics.queue.submit([]);
            player.graphics.device.poll(wgpu::PollType::Poll)?;
            maximum_update_time = maximum_update_time.max(poll_started.elapsed());

            if ready(state) && !state.seeking && player.displayed.is_some() {
                log::info!("Maximum UI playback call: {maximum_ui_time:?}");
                log::info!(
                    "Maximum UI update including queue submission: {maximum_update_time:?}"
                );
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

    fn resize(
        player: &mut Player,
        state: &mut VideoState,
        size: [u32; 2],
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        player.render(size)?;

        loop {
            player.poll(state);
            ensure!(state.error.is_none(), "Resize failed: {:?}", state.error);
            player.render(size)?;
            player.graphics.queue.submit([]);
            player.graphics.device.poll(wgpu::PollType::Poll)?;
            if player.target.as_ref().is_some_and(|target| {
                [target.texture.width(), target.texture.height()] == size
            }) {
                return Ok(());
            }

            ensure!(Instant::now() < deadline, "Video resize timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_video_pixels(player: &Player) -> Result<u64> {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let target = player
            .target
            .as_ref()
            .context("No video texture was displayed")?;
        let size = [target.texture.width(), target.texture.height()];
        let row_bytes = (size[0] * 4).div_ceil(256) * 256;
        let buffer = player
            .graphics
            .device
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("video-test-readback"),
                size: (row_bytes * size[1]) as u64,
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
                    rows_per_image: Some(size[1]),
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
            colorful > (size[0] * size[1] / 4) as usize,
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
