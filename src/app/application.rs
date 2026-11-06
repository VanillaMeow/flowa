//! Application composition, selection transitions, and the native lifecycle

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    services::{
        file_actions::Actions,
        folder_scan::FolderScanner,
        image_loader::{LoadRequest, Loader},
    },
    settings::{Settings, SettingsStore, Theme},
    ui::{
        self, ViewerUi,
        canvas::CanvasContent,
        keyboard,
        theme::{self, Colors},
        toolbar::{self, ToolbarState},
    },
    video::VideoSession,
    viewer::{browser::Browser, image_session::ImageSession, playback::Slideshow},
};

use super::{Command, Launch};

pub struct Flowa {
    pub(super) browser: Browser,
    pub(super) image: ImageSession,
    pub(super) video: VideoSession,
    pub(super) ui: ViewerUi,
    pub(super) settings: SettingsStore,
    pub(super) scanner: FolderScanner,
    pub(super) loader: Loader,
    pub(super) actions: Actions,
    pub(super) slideshow: Slideshow,
    pub(super) busy: Option<String>,

    pub(super) colors: Colors,
    pub(super) last_theme_check: Instant,
    pub(super) gpu: String,
    pub(super) initial_zoom: Option<f32>,
}

impl Flowa {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        settings: Settings,
        config_path: Option<PathBuf>,
        launch: Launch,
        video_device: bool,
    ) -> Self {
        let gpu = cc
            .wgpu_render_state
            .as_ref()
            .map(|state| {
                let info = state.adapter.get_info();
                format!("{} · {:?}", info.name, info.backend)
            })
            .unwrap_or_else(|| "wgpu".into());
        log::info!("Renderer: {gpu}");

        let mut app =
            Self::create(cc.egui_ctx.clone(), settings, config_path, launch, gpu);
        if video_device {
            app.video = VideoSession::new(cc);
        }

        app
    }

    pub(super) fn create(
        ctx: egui::Context,
        mut settings: Settings,
        config_path: Option<PathBuf>,
        launch: Launch,
        gpu: String,
    ) -> Self {
        settings.filmstrip |= launch.filmstrip;
        settings.info |= launch.info;

        let colors = theme::colors(
            settings.theme,
            settings.accent,
            ctx.system_theme().unwrap_or(egui::Theme::Dark) == egui::Theme::Dark,
        );
        theme::apply(&ctx, colors);
        ctx.set_zoom_factor(settings.ui_scale);

        let path = launch.path.or_else(|| {
            if settings.reopen_last {
                settings.recent.first().cloned()
            } else {
                None
            }
        });

        let mut app = Self {
            browser: Browser::new(),
            image: ImageSession::new(),
            video: VideoSession::default(),
            ui: ViewerUi::new(ctx.clone(), settings.nearest, launch.preferences),
            settings: SettingsStore::new(settings, config_path),
            scanner: FolderScanner::new(),
            loader: Loader::new(ctx.clone()),
            actions: Actions::new(ctx.clone()),
            slideshow: Slideshow::new(),
            busy: None,
            colors,
            last_theme_check: Instant::now(),
            gpu,
            initial_zoom: launch.zoom,
        };

        if let Some(path) = path {
            app.open_path(path, &ctx);
        }

        app
    }

    pub(super) fn open_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.video.close();
        self.ui.video_controls.reset();
        self.image.begin_load();
        self.loader.cancel(self.image.generation());
        self.ui
            .canvas
            .reset(self.image.generation(), self.settings.values.remember_view);

        let generation = self.browser.begin_scan();
        self.scanner.request(path, generation, ctx);
    }

    pub(super) fn clear_image(&mut self) {
        self.video.close();
        self.ui.video_controls.reset();
        self.image.clear();
        self.loader.cancel(self.image.generation());
        self.ui.canvas.clear(self.image.generation());
        self.slideshow.active = false;
    }

    pub(super) fn select(&mut self, index: usize, ctx: &egui::Context) {
        let Some(path) = self.browser.select(index) else {
            return;
        };

        let generation = self.image.begin_load();
        self.video.close();
        self.ui.video_controls.reset();
        self.loader.cancel(generation);
        self.ui
            .canvas
            .reset(generation, self.settings.values.remember_view);
        self.ui.filmstrip.reveal_selection();

        self.settings.values.remember(path.clone());

        if crate::viewer::catalog::is_video(&path) {
            self.image.loading = false;
            self.video.open(&path);
            self.slideshow.active = false;
            self.update_title(ctx);
            ctx.request_repaint();
            return;
        }

        self.loader.request(LoadRequest {
            generation,
            path,
            budget: self.settings.values.memory_mb * 1024 * 1024,
        });

        self.update_title(ctx);
        ctx.request_repaint();
    }

    pub(super) fn navigate(&mut self, delta: isize, ctx: &egui::Context) {
        if let Some(next) = self
            .browser
            .next(delta, self.settings.values.wrap_navigation)
        {
            self.select(next, ctx);
        }

        self.slideshow
            .restart(self.settings.values.slideshow_seconds);
    }

    pub(super) fn preload_image(&self, index: usize) {
        if index == self.browser.index() {
            return;
        }

        let Some(entry) = self.browser.entries().get(index) else {
            return;
        };

        if !crate::viewer::catalog::is_video(&entry.path) {
            self.loader.preload(entry.path.clone());
        }
    }

    pub(super) fn preload_neighbors(&self) {
        if self.image.loading {
            return;
        }

        let paths = [1, -1]
            .into_iter()
            .filter_map(|delta| {
                self.browser
                    .next(delta, self.settings.values.wrap_navigation)
            })
            .filter_map(|index| self.browser.entries().get(index))
            .filter(|entry| !crate::viewer::catalog::is_video(&entry.path))
            .map(|entry| entry.path.clone())
            .collect();

        self.loader.preload_neighbors(paths);
    }

    pub(super) fn update_title(&self, ctx: &egui::Context) {
        let title = self
            .browser
            .current()
            .map(|entry| format!("{} — flowa", entry.name))
            .unwrap_or_else(|| "flowa".into());

        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    fn toolbar_state(&self) -> ToolbarState<'_> {
        ToolbarState {
            browser: &self.browser,
            image: &self.image,
            video: &self.video.state,
            canvas: &self.ui.canvas,
            settings: &self.settings.values,
            colors: self.colors,
            fullscreen: self.ui.fullscreen,
            preferences_open: self.ui.preferences.open,
            busy: self.busy.as_deref(),
            slideshow: self.slideshow.active,
        }
    }

    fn refresh_theme(&mut self, ctx: &egui::Context) {
        self.colors = theme::colors(
            self.settings.values.theme,
            self.settings.values.accent,
            ctx.system_theme().unwrap_or(egui::Theme::Dark) == egui::Theme::Dark,
        );

        theme::apply(ctx, self.colors);
        self.last_theme_check = Instant::now();
    }

    fn persist(&mut self) {
        if let Err(error) = self.settings.persist() {
            self.ui.dialogs.notify(format!("{error:#}"), true);
        }
    }
}

impl eframe::App for Flowa {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.update_session(&ctx);
        self.show_viewer_controls(ui, &ctx);
        self.show_canvas(ui, &ctx);
        self.show_overlays(&ctx);
        self.maintain_settings(&ctx);

        ctx.request_repaint_after(Duration::from_secs(2));
    }

    fn on_exit(&mut self) {
        self.persist();
    }
}

impl Flowa {
    fn update_session(&mut self, ctx: &egui::Context) {
        self.ui.canvas.renderer.begin_pass();
        self.poll(ctx);
        self.video.poll(ctx);

        let commands =
            keyboard::read(ctx, self.ui.dialogs.is_open(), self.ui.preferences.open);
        self.handle_commands(commands, ctx);

        self.advance_playback(ctx);
        self.ui.canvas.renderer.sync_frame(ctx, &self.image);

        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        });

        if let Some(path) = dropped {
            self.handle(Command::OpenPath(path), ctx);
        }
    }

    fn advance_playback(&mut self, ctx: &egui::Context) {
        if let Some(delay) = self.image.animate(Instant::now()) {
            ctx.request_repaint_after(delay);
        }

        let ready = !self.image.loading && self.browser.entries().len() > 1;

        if let Some(delay) = self.slideshow.remaining(Instant::now(), ready) {
            if delay.is_zero() {
                self.navigate(1, ctx);
            }

            ctx.request_repaint_after(delay);
        }
    }

    fn show_viewer_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.ui.controls_visible(ctx, self.video.state.active) {
            return;
        }

        let mut commands = Vec::new();

        toolbar::show(ui, &self.toolbar_state(), &mut commands);
        self.handle_commands(std::mem::take(&mut commands), ctx);

        toolbar::status(ui, &self.toolbar_state());

        if self.settings.values.filmstrip {
            self.ui
                .filmstrip
                .show(ui, &self.browser, self.colors, &mut commands);
            self.handle_commands(std::mem::take(&mut commands), ctx);
        }

        // Bottom panels are allocated outside-in, so transport follows the filmstrip
        if self.video.state.active {
            self.ui.video_controls.show(
                ui,
                &self.video.state,
                self.colors,
                self.settings.values.compact_video_controls,
                &mut commands,
            );
            self.handle_commands(std::mem::take(&mut commands), ctx);
        }

        if self.settings.values.info {
            ui::inspector::show(
                ui,
                self.browser.current(),
                self.image.decoded.as_ref(),
                &self.video.state,
                self.colors,
                &mut commands,
            );
            self.handle_commands(commands, ctx);
        }
    }

    fn show_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut commands = Vec::new();
        let content = CanvasContent {
            image: &self.image,
            video: &self.video,
            settings: &self.settings.values,
            colors: self.colors,
            can_navigate: self.browser.entries().len() > 1,
            fullscreen: self.ui.fullscreen,
        };
        let response = self.ui.canvas.show(ui, content, &mut commands);

        if let Some(response) = response {
            response.context_menu(|ui| {
                toolbar::context_menu(ui, &self.toolbar_state(), &mut commands);
            });
        }

        self.handle_commands(commands, ctx);
    }

    fn show_overlays(&mut self, ctx: &egui::Context) {
        let old_nearest = self.settings.values.nearest;
        let old_memory = self.settings.values.memory_mb;
        let changed = self.ui.preferences.show(
            ctx,
            &mut self.settings.values,
            self.colors,
            &self.gpu,
            &self.video,
        );

        if changed {
            self.refresh_theme(ctx);

            if old_memory != self.settings.values.memory_mb {
                self.loader
                    .set_budget(self.settings.values.memory_mb * 1024 * 1024);
            }

            self.preload_neighbors();

            if old_nearest != self.settings.values.nearest {
                self.ui.canvas.renderer.set_sampling(
                    ctx,
                    &self.image,
                    self.settings.values.nearest,
                );
            }
        }

        let mut commands = Vec::new();

        self.ui.dialogs.show(ctx, self.colors, &mut commands);
        self.handle_commands(commands, ctx);
        self.ui.dialogs.show_notice(ctx, self.colors);
    }

    fn maintain_settings(&mut self, ctx: &egui::Context) {
        if self.last_theme_check.elapsed() > Duration::from_secs(2)
            && self.settings.values.theme == Theme::System
        {
            self.refresh_theme(ctx);
        }

        if self.settings.save_due() {
            self.persist();
        }

        let normal_window = !self.ui.fullscreen
            && !ctx.input(|input| input.viewport().maximized.unwrap_or(false));

        if normal_window
            && let Some(rect) = ctx.input(|input| input.viewport().inner_rect)
        {
            // Native window sizes exclude the application's UI zoom
            let native_size = rect.size() * ctx.zoom_factor();
            self.settings.values.window_size = [native_size.x, native_size.y];
        }
    }
}
