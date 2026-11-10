//! The background playback thread, which owns the pipeline and answers UI requests

use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

use eframe::egui_wgpu::RenderState;

use super::{engine::Snapshot, mailbox::Mailbox, pipeline::Pipeline};
use crate::video::{PlayerCommand, gpu::NativeGpu, presentation::Target};

/// One finished playback step, sent back to the UI
pub(super) struct Completed {
    pub generation: u64,
    pub status: Snapshot,
    pub target: Option<Target>,
    pub retired: Vec<Target>,
}

impl Completed {
    pub fn new(playback: &mut Pipeline, generation: u64) -> Self {
        Self {
            generation,
            status: playback.status.clone(),
            target: playback.displayed_target(),
            retired: playback.take_retired(),
        }
    }
}

/// Runs playback on this thread until the UI shuts it down, returning the textures
/// the UI must free
pub(super) fn run(
    _gpu: Arc<NativeGpu>,
    graphics: RenderState,
    ctx: egui::Context,
    requests: Arc<Mailbox>,
    results: mpsc::SyncSender<Completed>,
) -> Vec<Target> {
    // Construct the engine here so hardware frame pointers never cross threads
    let playback = match Pipeline::new(&graphics, &ctx) {
        Ok(playback) => playback,
        Err(error) => {
            log::error!("Could not start video playback: {error:#}");
            ctx.request_repaint();
            return Vec::new();
        }
    };

    let thread = PlaybackThread {
        playback,
        ctx,
        results,
        generation: 0,
        failure: None,
    };

    thread.run(&requests)
}

/// What the playback thread keeps between requests from the UI
struct PlaybackThread {
    playback: Pipeline,
    ctx: egui::Context,
    results: mpsc::SyncSender<Completed>,
    /// The source generation the current file was loaded for
    generation: u64,
    /// An error that stops rendering until the file or decoder changes
    failure: Option<anyhow::Error>,
}

impl PlaybackThread {
    /// Handles requests until shutdown, returning the targets the UI must free
    fn run(mut self, requests: &Mailbox) -> Vec<Target> {
        loop {
            let work = requests.take();

            if work.shutdown {
                break;
            }

            if let Some((generation, path)) = work.source {
                self.replace_source(generation, path);
            }

            let changed = !work.commands.is_empty();
            self.apply_commands(work.commands);

            let Some(size) = work.tick else {
                if changed {
                    self.ctx.request_repaint();
                }
                continue;
            };

            if !self.tick(size, changed) {
                break;
            }
        }

        self.finish()
    }

    fn replace_source(&mut self, generation: u64, path: Option<PathBuf>) {
        self.generation = generation;
        self.failure = match path {
            Some(path) => self.playback.load(&path).err(),
            None => {
                self.playback.stop();
                None
            }
        };
    }

    fn apply_commands(&mut self, commands: Vec<PlayerCommand>) {
        for command in commands {
            // Choosing another decoder is how the user retries after a failure
            if matches!(command, PlayerCommand::Decoder(_)) {
                self.failure = None;
            }

            if let Err(error) = self.playback.command(command) {
                self.failure = Some(error);
            }
        }
    }

    /// Advances and draws one step, returning false once the UI stops listening
    fn tick(&mut self, size: Option<[u32; 2]>, changed: bool) -> bool {
        let status = &self.playback.status;
        let previous_serial = status.frame_serial;
        let previous_error = status.video.error.clone();
        let previous_displayed = self.playback.displayed();

        self.playback.poll();
        self.render(size);

        let displayed = self.playback.displayed();
        let video = &mut self.playback.status.video;
        if let Some(error) = &self.failure {
            video.error = Some(format!("{error:#}"));
        }

        let notify = changed
            || previous_serial != self.playback.status.frame_serial
            || previous_displayed != displayed
            || previous_error != video.error;
        let active =
            video.error.is_none() && (video.loading || video.seeking || !video.paused);
        let next = delay(&self.playback.status);

        let completed = Completed::new(&mut self.playback, self.generation);
        if self.results.send(completed).is_err() {
            return false;
        }

        if notify {
            self.ctx.request_repaint();
        } else if active {
            self.ctx.request_repaint_after(next);
        }

        true
    }

    /// Draws the latest frame unless playback has failed or shown nothing yet
    fn render(&mut self, size: Option<[u32; 2]>) {
        let status = &self.playback.status;

        if self.failure.is_some()
            || status.video.error.is_some()
            || status.frame_serial == 0
        {
            return;
        }

        let size = size.unwrap_or(status.video.size);
        if let Err(error) = self.playback.render(size) {
            self.failure = Some(error);
        }
    }

    fn finish(mut self) -> Vec<Target> {
        // The UI frees these textures after the join, once the pipeline is dropped
        self.playback.finish()
    }
}

/// How long the UI may sleep before the next frame is due, kept within 1–100 ms
pub(super) fn delay(status: &Snapshot) -> Duration {
    let seconds = status.next_frame_delay;
    Duration::from_secs_f64(if seconds.is_finite() {
        seconds.clamp(0.001, 0.1)
    } else {
        0.01
    })
}
