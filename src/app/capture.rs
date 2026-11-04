//! Native screenshots used by development smoke tests

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use crate::{
    ui::rendering::image_renderer::ImageRenderer, video::VideoSession,
    viewer::image_session::ImageSession,
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
        video: &VideoSession,
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

        let waiting_for_video = video.state.active
            && video.state.error.is_none()
            && (video.state.loading || video.texture.is_none());

        if waiting_for_video || renderer.has_pending_tiles() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }

        self.requested = true;
        log::info!(
            "Screenshot ready: {} detailed tiles, {} files in folder",
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::Capture;
    use crate::{
        ui::rendering::image_renderer::ImageRenderer, video::VideoSession,
        viewer::image_session::ImageSession,
    };

    #[test]
    fn video_screenshots_wait_for_a_frame_and_allow_error_capture() {
        let ctx = egui::Context::default();
        let renderer = ImageRenderer::new(ctx.clone(), false);
        let image = ImageSession::new();
        let mut video = VideoSession::default();
        video.state.active = true;
        video.state.loading = true;

        let mut capture = Capture::new(Some("unused-screenshot.png".into()), true);
        capture.started = Instant::now() - Duration::from_secs(3);
        let update = |capture: &mut Capture, video: &VideoSession| {
            let mut output = ctx.run_ui(Default::default(), |_| {
                capture.update(&ctx, &image, video, &renderer, 1);
            });
            output.textures_delta.clear();
        };

        update(&mut capture, &video);
        assert!(!capture.requested);

        video.state.loading = false;
        update(&mut capture, &video);
        assert!(!capture.requested);

        video.texture = Some(egui::TextureId::User(1));
        update(&mut capture, &video);
        assert!(capture.requested);

        capture.requested = false;
        video.texture = None;
        video.state.error = Some("Decoder failed".into());
        update(&mut capture, &video);
        assert!(capture.requested);
    }
}
