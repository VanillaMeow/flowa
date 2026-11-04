//! Coordinate worker results that affect multiple owned components

use egui::TextureOptions;

use crate::services::{file_actions::Outcome, folder_scan};

use super::Flowa;

impl Flowa {
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        self.poll_catalog(ctx);
        self.poll_images(ctx);
        self.ui.canvas.renderer.poll(ctx, self.image.generation());
        self.ui.filmstrip.poll(ctx);

        while let Ok(outcome) = self.actions.results.try_recv() {
            self.busy = None;

            match outcome {
                Ok(outcome) => self.apply_outcome(outcome, ctx),
                Err(error) => self.ui.dialogs.notify(format!("{error:#}"), true),
            }
        }
    }

    fn poll_catalog(&mut self, ctx: &egui::Context) {
        while let Some(result) = self.scanner.poll(self.browser.generation()) {
            match result {
                Ok(folder) => {
                    self.browser.replace(
                        folder,
                        self.settings.values.sort,
                        self.settings.values.reverse_sort,
                    );
                    self.ui.filmstrip.reset(self.browser.generation());

                    if self.browser.entries().is_empty() {
                        self.clear_image();
                        self.image.error =
                            Some("There are no supported files in this folder.".into());
                    } else {
                        self.select(self.browser.index(), ctx);
                    }
                }
                Err(error) => {
                    self.image.loading = false;
                    self.ui.dialogs.notify(format!("{error:#}"), true);
                }
            }
        }
    }

    fn poll_images(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.loader.results.try_recv() {
            if !self.image.accept(result.generation, result.result) {
                continue;
            }

            if let Some(decoded) = &self.image.decoded {
                self.ui.canvas.renderer.sync_frame(ctx, &self.image);

                if let Some(entry) = self.browser.current() {
                    let thumbnail = ctx.load_texture(
                        format!("thumbnail:{}", entry.path.display()),
                        decoded.thumbnail.clone(),
                        TextureOptions::LINEAR,
                    );
                    self.ui.filmstrip.cache(entry.path.clone(), Some(thumbnail));
                }

                if let Some(percent) = self.initial_zoom.take() {
                    self.ui.canvas.view.fit = false;
                    self.ui.canvas.view.zoom = percent / 100.0 / ctx.pixels_per_point();
                }

                self.slideshow
                    .restart(self.settings.values.slideshow_seconds);
                self.preload_neighbors();
            }
        }
    }

    fn apply_outcome(&mut self, outcome: Outcome, ctx: &egui::Context) {
        match outcome {
            Outcome::Picked(Some(path)) => self.open_path(path, ctx),
            Outcome::Picked(None) | Outcome::Revealed => {}
            Outcome::Copied => self.ui.dialogs.notify("Image copied", false),
            Outcome::Exported => self.ui.dialogs.notify("Copy exported", false),
            Outcome::Renamed { old, new } => {
                self.loader.invalidate();
                self.settings.values.recent.retain(|path| *path != old);

                let entry = match folder_scan::entry_from_path(new.clone()) {
                    Ok(entry) => entry,
                    Err(error) => {
                        self.ui.dialogs.notify(format!("{error:#}"), true);
                        return;
                    }
                };

                self.browser.rename(
                    &old,
                    entry,
                    self.settings.values.sort,
                    self.settings.values.reverse_sort,
                );
                self.ui.filmstrip.remove(&old);
                self.ui.filmstrip.reveal_selection();
                self.settings.values.remember(new);
                self.update_title(ctx);
                self.ui.dialogs.notify("Image renamed", false);

                if self.image.loading {
                    self.select(self.browser.index(), ctx);
                }
            }
            Outcome::Trashed(path) => {
                self.loader.invalidate();
                self.settings.values.recent.retain(|recent| *recent != path);
                self.ui.filmstrip.remove(&path);

                if let Some(index) = self.browser.remove(&path) {
                    if self.browser.entries().is_empty() {
                        self.clear_image();
                        self.update_title(ctx);
                    } else {
                        self.select(index, ctx);
                    }
                }

                self.ui.dialogs.notify("Moved to the system trash", false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use anyhow::Result;
    use egui::vec2;

    use crate::{
        app::{Command, Flowa, Launch},
        services::{file_actions::Outcome, image_decode::decode},
        settings::{Settings, Theme},
    };

    fn app_with_images(
        count: usize,
    ) -> Result<(tempfile::TempDir, egui::Context, Flowa)> {
        let directory = tempfile::tempdir()?;
        for index in 0..count {
            image::RgbaImage::new(2, 2)
                .save(directory.path().join(format!("{index}.png")))?;
        }
        let ctx = egui::Context::default();
        let launch = Launch {
            path: None,
            screenshot: None,
            smoke_test: true,
            preferences: false,
            filmstrip: false,
            info: false,
            zoom: None,
        };
        let mut app = Flowa::create(
            ctx.clone(),
            Settings {
                theme: Theme::Nord,
                ..Default::default()
            },
            None,
            launch,
            "Test".into(),
        );
        app.open_path(directory.path().join("0.png"), &ctx);
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.browser.entries().is_empty() {
            app.poll_catalog(&ctx);
            anyhow::ensure!(Instant::now() < deadline, "Folder scan timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok((directory, ctx, app))
    }

    #[test]
    fn selecting_another_image_resets_view_unless_remembered() -> Result<()> {
        let (_directory, ctx, mut app) = app_with_images(2)?;
        let generation = app.image.generation();
        app.ui.canvas.view.rotation = 1;
        app.ui.canvas.view.pan = vec2(30., 20.);
        app.ui.canvas.view.fit = false;
        app.ui.canvas.view.zoom = 2.;
        app.handle(Command::Navigate(1), &ctx);

        assert_eq!(app.browser.index(), 1);
        assert!(app.image.generation() > generation);
        assert!(app.image.loading);
        assert!(app.ui.canvas.view.fit);
        assert_eq!(app.ui.canvas.view.rotation, 0);
        assert_eq!(app.ui.canvas.view.pan, egui::Vec2::ZERO);

        app.settings.values.remember_view = true;
        app.ui.canvas.view.rotation = 3;
        app.ui.canvas.view.zoom = 2.;
        app.handle(Command::Navigate(-1), &ctx);
        assert_eq!(app.browser.index(), 0);
        assert_eq!(app.ui.canvas.view.rotation, 3);
        assert_eq!(app.ui.canvas.view.zoom, 2.);
        Ok(())
    }

    #[test]
    fn trashing_the_last_image_invalidates_pending_loads() -> Result<()> {
        let (directory, ctx, mut app) = app_with_images(1)?;
        // File actions receive canonical paths from the scanned catalog
        let path = directory.path().join("0.png").canonicalize()?;
        let generation = app.image.generation();
        let pending = decode(&path, 1024 * 1024)?;
        app.slideshow.active = true;
        assert!(app.image.loading);
        assert!(app.settings.values.recent.contains(&path));

        app.apply_outcome(Outcome::Trashed(path), &ctx);

        assert!(app.browser.current().is_none());
        assert!(app.image.generation() > generation);
        assert!(!app.image.loading);
        assert!(!app.slideshow.active);
        assert!(app.settings.values.recent.is_empty());
        assert!(!app.image.accept(generation, Ok(pending)));
        assert!(app.image.decoded.is_none());
        Ok(())
    }

    #[test]
    fn renaming_during_loading_restarts_decode_with_the_new_path() -> Result<()> {
        let (directory, ctx, mut app) = app_with_images(1)?;
        let old = directory.path().join("0.png").canonicalize()?;
        let new = old.with_file_name("renamed.png");
        let generation = app.image.generation();
        std::fs::rename(&old, &new)?;
        app.apply_outcome(
            Outcome::Renamed {
                old: old.clone(),
                new: new.clone(),
            },
            &ctx,
        );

        assert!(app.image.generation() > generation);
        assert_eq!(app.browser.current().map(|entry| &entry.path), Some(&new));
        assert!(!app.settings.values.recent.contains(&old));
        assert!(app.settings.values.recent.contains(&new));
        loop {
            let result = app.loader.results.recv_timeout(Duration::from_secs(3))?;
            if result.generation == app.image.generation() {
                assert!(result.result.is_ok());
                break;
            }
        }
        Ok(())
    }
}
