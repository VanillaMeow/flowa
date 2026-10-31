//! Native screenshots used by development smoke tests

use super::*;

impl Flowa {
    pub(super) fn screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot_requested
            || self.started.elapsed() < Duration::from_secs(2)
            || self.loading
        {
            return;
        }
        let Some(path) = self.launch.screenshot.clone() else {
            if self.launch.smoke_test && self.started.elapsed() > Duration::from_secs(3)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        };
        if !self.pending_tiles.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        self.screenshot_requested = true;
        log::info!(
            "Screenshot ready: {} detailed tiles, {} images in folder",
            self.tiles.len(),
            self.files.len()
        );
        let close = self.launch.smoke_test;
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
