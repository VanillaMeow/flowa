//! Native screenshots used by development smoke tests

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use crate::{
    ui::rendering::image_renderer::ImageRenderer, viewer::image_session::ImageSession,
};

pub(super) struct Capture {
    path: Option<PathBuf>,
    smoke_test: bool,
    started: Instant,
    requested: bool,
}

impl Capture {
    pub fn new(path: Option<PathBuf>, smoke_test: bool) -> Self {
        Self {
            path,
            smoke_test,
            started: Instant::now(),
            requested: false,
        }
    }

    pub fn update(
        &mut self,
        ctx: &egui::Context,
        image: &ImageSession,
        renderer: &ImageRenderer,
        file_count: usize,
    ) {
        if self.requested
            || self.started.elapsed() < Duration::from_secs(2)
            || image.loading
        {
            return;
        }

        let Some(path) = self.path.clone() else {
            if self.smoke_test && self.started.elapsed() > Duration::from_secs(3) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        };

        if renderer.has_pending_tiles() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }

        self.requested = true;
        log::info!(
            "Screenshot ready: {} detailed tiles, {} images in folder",
            renderer.tile_count(),
            file_count
        );
        let close = self.smoke_test;
        let ctx2 = ctx.clone();

        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
            egui::ScreenshotCallback::new(move |image| {
                let bytes: Vec<u8> =
                    image.pixels.iter().flat_map(|p| p.to_array()).collect();

                let result = (|| -> Result<()> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    image::save_buffer(
                        &path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .context("Could not save screenshot")?;
                    Ok(())
                })();

                match result {
                    Ok(()) => println!("Screenshot saved: {}", path.display()),
                    Err(error) => eprintln!("Screenshot failed: {error:#}"),
                }

                if close {
                    ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }),
        ));
    }
}
