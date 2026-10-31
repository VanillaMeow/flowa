//! Application state, file selection, and the native window lifecycle

mod canvas;
mod capture;
mod dialogs;
mod events;
mod filmstrip;
mod input;
mod inspector;
mod toolbar;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use egui::{
    Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
    pos2, vec2,
};

use crate::{
    actions::{Actions, Outcome, Task},
    catalog::{self, Entry},
    icons::{self, Icon},
    loader::{
        self, DecodedImage, LoadRequest, Loader, ThumbnailRequest, ThumbnailWorker,
        TileKey, TileRequest, TileWorker,
    },
    preferences,
    settings::{Canvas, Settings, SortOrder},
    theme::{self, Colors},
    view::View,
};

pub struct Launch {
    pub path: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub smoke_test: bool,
    pub preferences: bool,
    pub filmstrip: bool,
    pub info: bool,
    pub zoom: Option<f32>,
}

struct CatalogResult {
    serial: u64,
    folder: PathBuf,
    selected: Option<PathBuf>,
    result: Result<Vec<Entry>>,
}

struct TileTexture {
    texture: TextureHandle,
    source: Rect,
    touched: u64,
}

struct Notice {
    message: String,
    error: bool,
    until: Instant,
}

enum Dialog {
    Rename { path: PathBuf, name: String },
    Trash(PathBuf),
}

pub struct Flowa {
    settings: Settings,
    config_path: Option<PathBuf>,
    saved_settings: String,
    last_save: Instant,

    colors: Colors,
    last_theme_check: Instant,
    gpu: String,

    files: Vec<Entry>,
    folder: Option<PathBuf>,
    index: usize,
    scan_serial: u64,
    catalog_tx: mpsc::Sender<CatalogResult>,
    catalog_rx: mpsc::Receiver<CatalogResult>,

    loader: Loader,
    tile_worker: TileWorker,
    thumbnail_worker: ThumbnailWorker,
    actions: Actions,

    generation: u64,
    decoded: Option<DecodedImage>,
    loading: bool,
    load_error: Option<String>,

    preview_texture: Option<TextureHandle>,
    preview_frame: usize,
    frame_index: usize,
    frame_deadline: Instant,
    playing: bool,

    tiles: HashMap<TileKey, TileTexture>,
    pending_tiles: HashSet<TileKey>,
    paint_counter: u64,

    thumbnails: HashMap<PathBuf, Option<TextureHandle>>,
    thumb_pending: HashSet<PathBuf>,
    thumb_order: Vec<PathBuf>,

    view: View,
    canvas_rect: Rect,
    fullscreen: bool,

    preferences_open: bool,
    preferences_tab: usize,
    dialog: Option<Dialog>,
    notice: Option<Notice>,
    busy: Option<String>,

    slideshow: bool,
    next_slide: Instant,
    scroll_to_current: bool,

    launch: Launch,
    started: Instant,
    screenshot_requested: bool,
}

impl Flowa {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        mut settings: Settings,
        config_path: Option<PathBuf>,
        launch: Launch,
    ) -> Self {
        let ctx = cc.egui_ctx.clone();
        settings.filmstrip |= launch.filmstrip;
        settings.info |= launch.info;

        let colors = theme::colors(
            settings.theme,
            settings.accent,
            ctx.system_theme().unwrap_or(egui::Theme::Dark) == egui::Theme::Dark,
        );
        theme::apply(&ctx, colors, settings.ui_scale);

        let gpu = cc
            .wgpu_render_state
            .as_ref()
            .map(|state| {
                let info = state.adapter.get_info();
                format!("{} · {:?}", info.name, info.backend)
            })
            .unwrap_or_else(|| "wgpu".into());
        log::info!("Renderer: {gpu}");
        let (catalog_tx, catalog_rx) = mpsc::channel();
        let now = Instant::now();
        let path = launch.path.clone().or_else(|| {
            if settings.reopen_last {
                settings.recent.first().cloned()
            } else {
                None
            }
        });

        let mut app = Self {
            saved_settings: serde_json::to_string(&settings).unwrap_or_default(),
            settings,
            config_path,
            last_save: now,
            colors,
            last_theme_check: now,
            gpu,
            files: Vec::new(),
            folder: None,
            index: 0,
            scan_serial: 0,
            catalog_tx,
            catalog_rx,
            loader: Loader::new(ctx.clone()),
            tile_worker: TileWorker::new(ctx.clone()),
            thumbnail_worker: ThumbnailWorker::new(ctx.clone()),
            actions: Actions::new(ctx.clone()),
            generation: 0,
            decoded: None,
            loading: false,
            load_error: None,
            preview_texture: None,
            preview_frame: usize::MAX,
            frame_index: 0,
            frame_deadline: now,
            playing: true,
            tiles: HashMap::new(),
            pending_tiles: HashSet::new(),
            paint_counter: 0,
            thumbnails: HashMap::new(),
            thumb_pending: HashSet::new(),
            thumb_order: Vec::new(),
            view: View::default(),
            canvas_rect: Rect::NOTHING,
            fullscreen: false,
            preferences_open: launch.preferences,
            preferences_tab: 0,
            dialog: None,
            notice: None,
            busy: None,
            slideshow: false,
            next_slide: now,
            scroll_to_current: false,
            started: now,
            screenshot_requested: false,
            launch,
        };

        if let Some(path) = path {
            app.open_path(path, &ctx);
        }

        app
    }

    fn current(&self) -> Option<&Entry> {
        self.files.get(self.index)
    }

    fn clear_image(&mut self) {
        // Invalidate results that were queued before the image was cleared
        self.generation += 1;
        self.loader.cancel(self.generation);
        self.tile_worker
            .generation
            .store(self.generation, Ordering::Relaxed);

        self.decoded = None;
        self.preview_texture = None;
        self.tiles.clear();
        self.pending_tiles.clear();
        self.loading = false;
        self.slideshow = false;
    }

    fn current_pixels(&self) -> Option<std::sync::Arc<image::RgbaImage>> {
        self.decoded
            .as_ref()?
            .frames
            .get(self.frame_index)
            .map(|f| f.pixels.clone())
    }

    fn notify(&mut self, message: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            message: message.into(),
            error,
            until: Instant::now() + Duration::from_secs(if error { 12 } else { 4 }),
        });
    }

    fn texture_options(&self) -> TextureOptions {
        if self.settings.nearest {
            TextureOptions::NEAREST
        } else {
            TextureOptions::LINEAR
        }
    }

    fn open_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.scan_serial += 1;
        let serial = self.scan_serial;
        self.loading = true;
        self.load_error = None;

        let tx = self.catalog_tx.clone();
        let ctx = ctx.clone();

        std::thread::spawn(move || {
            let path = path.canonicalize().unwrap_or(path);
            let selected = if path.is_dir() {
                None
            } else {
                Some(path.clone())
            };
            let folder = if path.is_dir() {
                path.clone()
            } else {
                path.parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_path_buf()
            };

            let result = (|| {
                let mut files = catalog::scan(&folder)?;
                if let Some(ref selected) = selected
                    && !files.iter().any(|f| f.path == *selected)
                {
                    // An explicitly opened file may have a nonstandard extension
                    files.push(Entry::from_path(selected.clone())?);
                }
                Ok(files)
            })();

            let _ = tx.send(CatalogResult {
                serial,
                folder,
                selected,
                result,
            });
            ctx.request_repaint();
        });
    }

    fn select(&mut self, index: usize, ctx: &egui::Context) {
        let Some(path) = self.files.get(index).map(|f| f.path.clone()) else {
            return;
        };
        self.index = index;
        self.generation += 1;
        self.loading = true;
        self.load_error = None;
        self.decoded = None;
        self.preview_texture = None;
        self.preview_frame = usize::MAX;
        self.frame_index = 0;
        self.tiles.clear();
        self.pending_tiles.clear();
        self.tile_worker
            .generation
            .store(self.generation, Ordering::Relaxed);

        if !self.settings.remember_view {
            self.view = View::default();
        }

        self.scroll_to_current = true;
        self.settings.remember(path.clone());
        self.loader.request(LoadRequest {
            generation: self.generation,
            path,
            budget: self.settings.memory_mb * 1024 * 1024,
        });
        if let Some(entry) = self.current() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "{} — flowa",
                entry.name
            )));
        }
        ctx.request_repaint();
    }

    fn navigate(&mut self, delta: isize, ctx: &egui::Context) {
        if let Some(next) = catalog::next_index(
            self.index,
            self.files.len(),
            delta,
            self.settings.wrap_navigation,
        ) && next != self.index
        {
            self.select(next, ctx);
        }
        self.next_slide =
            Instant::now() + Duration::from_secs_f32(self.settings.slideshow_seconds);
    }

    fn pick(&mut self, folder: bool) {
        if self.busy.is_none() {
            self.busy = Some("Opening…".into());
            self.actions.request(Task::Pick {
                folder,
                start: self.folder.clone(),
            });
        }
    }

    fn copy_image(&mut self) {
        if self.busy.is_some() {
            return;
        }
        if let Some(pixels) = self.current_pixels() {
            self.busy = Some("Copying image…".into());
            self.actions.request(Task::Copy {
                pixels,
                view: self.view.clone(),
            });
        }
    }

    fn export(&mut self) {
        if self.busy.is_some() {
            return;
        }
        if let (Some(pixels), Some(entry)) = (self.current_pixels(), self.current()) {
            self.actions.request(Task::Export {
                pixels,
                source: entry.path.clone(),
                view: self.view.clone(),
            });
            self.busy = Some("Exporting a copy…".into());
        }
    }

    fn rename_dialog(&mut self) {
        if self.busy.is_none()
            && let Some(entry) = self.current()
        {
            self.dialog = Some(Dialog::Rename {
                path: entry.path.clone(),
                name: entry.name.clone(),
            });
        }
    }

    fn trash_dialog(&mut self) {
        if self.busy.is_none()
            && let Some(entry) = self.current()
        {
            self.dialog = Some(Dialog::Trash(entry.path.clone()));
        }
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    fn toggle_play(&mut self) {
        if self.decoded.as_ref().is_some_and(|i| i.frames.len() > 1) {
            self.playing = !self.playing;
            self.frame_deadline = Instant::now();
        } else {
            self.slideshow = !self.slideshow;
            self.next_slide = Instant::now()
                + Duration::from_secs_f32(self.settings.slideshow_seconds);
        }
    }

    fn sort(&mut self) {
        let selected = self.current().map(|f| f.path.clone());
        catalog::sort(
            &mut self.files,
            self.settings.sort,
            self.settings.reverse_sort,
        );
        if let Some(path) = selected {
            self.index = self.files.iter().position(|f| f.path == path).unwrap_or(0);
        }
        self.scroll_to_current = true;
    }

    fn refresh_theme(&mut self, ctx: &egui::Context) {
        self.colors = theme::colors(
            self.settings.theme,
            self.settings.accent,
            ctx.system_theme().unwrap_or(egui::Theme::Dark) == egui::Theme::Dark,
        );
        theme::apply(ctx, self.colors, self.settings.ui_scale);
        self.last_theme_check = Instant::now();
    }

    fn persist(&mut self) {
        let snapshot = serde_json::to_string(&self.settings).unwrap_or_default();
        if snapshot != self.saved_settings
            && let Some(path) = self.config_path.as_ref()
        {
            match self.settings.save(path) {
                Ok(()) => self.saved_settings = snapshot,
                Err(e) => self.notify(format!("{e:#}"), true),
            }
        }
        self.last_save = Instant::now();
    }

    fn animation(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(decoded) = &self.decoded
            && decoded.frames.len() > 1
            && self.playing
        {
            if now >= self.frame_deadline {
                self.frame_index = (self.frame_index + 1) % decoded.frames.len();
                self.frame_deadline = now + decoded.frames[self.frame_index].delay;
            }
            ctx.request_repaint_after(
                self.frame_deadline.saturating_duration_since(now),
            );
        }
        if self.slideshow && !self.loading && self.files.len() > 1 {
            if now >= self.next_slide {
                self.navigate(1, ctx);
            }
            ctx.request_repaint_after(self.next_slide.saturating_duration_since(now));
        }
        if self.preview_frame != self.frame_index
            && let Some(decoded) = &self.decoded
            && let Some(frame) = decoded.frames.get(self.frame_index)
        {
            let options = self.texture_options();
            if let Some(texture) = self.preview_texture.as_mut() {
                texture.set(frame.preview.clone(), options);
            }
            self.preview_frame = self.frame_index;
        }
    }
}

impl eframe::App for Flowa {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.paint_counter = self.paint_counter.wrapping_add(1);

        self.poll(&ctx);
        self.keyboard(&ctx);
        self.animation(&ctx);

        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if let Some(path) = dropped.first() {
            self.open_path(path.clone(), &ctx);
        }

        let controls = !self.fullscreen
            || self.preferences_open
            || self.dialog.is_some()
            || ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y < 60.));
        if controls {
            self.toolbar(ui);
            self.status(ui);
            if self.settings.filmstrip {
                self.filmstrip(ui);
            }
            if self.settings.info {
                self.inspector(ui);
            }
        }

        self.canvas(ui);

        let old_nearest = self.settings.nearest;
        if self.preferences_open
            && preferences::show(
                &ctx,
                &mut self.preferences_open,
                &mut self.settings,
                &mut self.preferences_tab,
                self.colors,
                &self.gpu,
            )
        {
            self.refresh_theme(&ctx);
            if old_nearest != self.settings.nearest {
                let options = self.texture_options();
                self.tiles.clear();
                if let Some(decoded) = &self.decoded
                    && let Some(texture) = self.preview_texture.as_mut()
                {
                    texture
                        .set(decoded.frames[self.frame_index].preview.clone(), options);
                }
            }
        }

        self.dialogs(&ctx);
        self.notice(&ctx);
        if self.last_theme_check.elapsed() > Duration::from_secs(2)
            && self.settings.theme == theme::Theme::System
        {
            self.refresh_theme(&ctx);
        }
        if self.last_save.elapsed() > Duration::from_secs(2) {
            self.persist();
        }
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect)
            && !self.fullscreen
            && !ctx.input(|i| i.viewport().maximized.unwrap_or(false))
        {
            self.settings.window_size = [rect.width(), rect.height()];
        }

        self.screenshot(&ctx);
        ctx.request_repaint_after(Duration::from_secs(2));
    }

    fn on_exit(&mut self) {
        self.persist();
    }
}
