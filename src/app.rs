use crate::{
    actions::{Actions, Outcome, Task},
    catalog::{self, Entry},
    icons::{self, Icon},
    loader::{
        self, DecodedImage, LoadRequest, Loader, ThumbnailRequest, ThumbnailWorker, TileKey,
        TileRequest, TileWorker,
    },
    preferences,
    settings::{Canvas, Settings, SortOrder},
    theme::{self, Colors},
    view::View,
};
use anyhow::{Context, Result};
use egui::{
    Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
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
        self.next_slide = Instant::now() + Duration::from_secs_f32(self.settings.slideshow_seconds);
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
            self.next_slide =
                Instant::now() + Duration::from_secs_f32(self.settings.slideshow_seconds);
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
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.catalog_rx.try_recv() {
            if result.serial != self.scan_serial {
                continue;
            }
            match result.result {
                Ok(mut files) => {
                    catalog::sort(&mut files, self.settings.sort, self.settings.reverse_sort);
                    let index = result
                        .selected
                        .and_then(|p| files.iter().position(|f| f.path == p))
                        .unwrap_or(0);
                    self.files = files;
                    self.folder = Some(result.folder);
                    self.thumbnails.clear();
                    self.thumb_order.clear();
                    self.thumb_pending.clear();
                    self.thumbnail_worker
                        .generation
                        .store(self.scan_serial, Ordering::Relaxed);
                    if self.files.is_empty() {
                        self.index = 0;
                        self.clear_image();
                        self.load_error =
                            Some("There are no supported images in this folder.".into());
                    } else {
                        self.select(index, ctx);
                    }
                }
                Err(error) => {
                    self.loading = false;
                    self.notify(format!("{error:#}"), true);
                }
            }
        }
        while let Ok(result) = self.loader.results.try_recv() {
            if result.generation != self.generation {
                continue;
            }
            self.loading = false;
            match result.result {
                Ok(decoded) => {
                    let first = &decoded.frames[0];
                    self.preview_texture = Some(ctx.load_texture(
                        "image-preview",
                        first.preview.clone(),
                        self.texture_options(),
                    ));
                    self.preview_frame = 0;
                    self.frame_index = 0;
                    self.playing = true;
                    self.frame_deadline = Instant::now() + first.delay;
                    if let Some(path) = self.current().map(|e| e.path.clone()) {
                        let thumbnail = ctx.load_texture(
                            format!("thumbnail:{}", path.display()),
                            decoded.thumbnail.clone(),
                            TextureOptions::LINEAR,
                        );
                        self.cache_thumbnail(path, Some(thumbnail));
                    }
                    self.decoded = Some(decoded);
                    if let Some(percent) = self.launch.zoom.take() {
                        self.view.fit = false;
                        self.view.zoom = percent / 100.0 / ctx.pixels_per_point();
                    }
                    self.next_slide =
                        Instant::now() + Duration::from_secs_f32(self.settings.slideshow_seconds);
                }
                Err(error) => {
                    self.load_error = Some(format!("{error:#}"));
                    self.decoded = None;
                }
            }
        }
        // Bound texture work per frame so decoding a large image does not stall input.
        for _ in 0..4 {
            let Ok(result) = self.tile_worker.results.try_recv() else {
                break;
            };
            self.pending_tiles.remove(&result.key);
            if result.key.generation != self.generation {
                continue;
            }
            let texture = ctx.load_texture(
                format!("tile:{:?}", result.key),
                result.image,
                self.texture_options(),
            );
            self.tiles.insert(
                result.key,
                TileTexture {
                    texture,
                    source: result.source,
                    touched: self.paint_counter,
                },
            );
            while self.tiles.len() > 46 {
                if let Some(key) = self
                    .tiles
                    .iter()
                    .min_by_key(|(_, v)| v.touched)
                    .map(|(k, _)| *k)
                {
                    self.tiles.remove(&key);
                } else {
                    break;
                }
            }
            ctx.request_repaint();
        }
        for _ in 0..4 {
            let Ok(result) = self.thumbnail_worker.results.try_recv() else {
                break;
            };
            self.thumb_pending.remove(&result.path);
            if result.generation != self.scan_serial {
                continue;
            }
            let texture = result.image.map(|image| {
                ctx.load_texture(
                    format!("thumb:{}", result.path.display()),
                    image,
                    TextureOptions::LINEAR,
                )
            });
            self.cache_thumbnail(result.path, texture);
        }
        while let Ok(outcome) = self.actions.results.try_recv() {
            self.busy = None;
            match outcome {
                Ok(Outcome::Picked(Some(path))) => self.open_path(path, ctx),
                Ok(Outcome::Picked(None) | Outcome::Revealed) => {}
                Ok(Outcome::Copied) => self.notify("Image copied", false),
                Ok(Outcome::Exported) => self.notify("Copy exported", false),
                Ok(Outcome::Renamed { old, new }) => {
                    self.settings.recent.retain(|p| *p != old);
                    if let Some(entry) = self.files.iter_mut().find(|f| f.path == old)
                        && let Ok(new_entry) = Entry::from_path(new.clone())
                    {
                        *entry = new_entry;
                    }
                    self.thumbnails.remove(&old);
                    self.sort();
                    self.settings.remember(new);
                    if let Some(entry) = self.current() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                            "{} — flowa",
                            entry.name
                        )));
                    }
                    self.notify("Image renamed", false);
                    if self.loading {
                        self.select(self.index, ctx);
                    }
                }
                Ok(Outcome::Trashed(path)) => {
                    self.settings.recent.retain(|p| *p != path);
                    self.thumbnails.remove(&path);
                    if let Some(i) = self.files.iter().position(|f| f.path == path) {
                        self.files.remove(i);
                        if self.files.is_empty() {
                            self.clear_image();
                            self.index = 0;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Title("flowa".into()));
                        } else {
                            self.select(i.min(self.files.len() - 1), ctx);
                        }
                    }
                    self.notify("Moved to the system trash", false);
                }
                Err(error) => self.notify(format!("{error:#}"), true),
            }
        }
    }
    fn cache_thumbnail(&mut self, path: PathBuf, texture: Option<TextureHandle>) {
        self.thumb_order.retain(|p| *p != path);
        self.thumb_order.push(path.clone());
        self.thumbnails.insert(path, texture);
        while self.thumb_order.len() > 128 {
            let old = self.thumb_order.remove(0);
            self.thumbnails.remove(&old);
        }
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
            ctx.request_repaint_after(self.frame_deadline.saturating_duration_since(now));
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
    fn keyboard(&mut self, ctx: &egui::Context) {
        let key = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        if key(egui::Modifiers::CTRL, egui::Key::Q) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::O) {
            self.pick(true);
        } else if key(egui::Modifiers::CTRL, egui::Key::O) {
            self.pick(false);
        }
        if key(egui::Modifiers::CTRL, egui::Key::Comma) {
            self.preferences_open = !self.preferences_open;
        }
        if self.dialog.is_some() {
            return;
        }
        if key(egui::Modifiers::NONE, egui::Key::Escape) {
            if self.preferences_open {
                self.preferences_open = false;
            } else if self.fullscreen {
                self.toggle_fullscreen(ctx);
            } else {
                self.slideshow = false;
            }
            return;
        }
        if ctx.text_edit_focused() || self.preferences_open || egui::Popup::is_any_open(ctx) {
            return;
        }
        if key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
            self.navigate(1, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::F5)
            && let Some(path) = self
                .current()
                .map(|e| e.path.clone())
                .or_else(|| self.folder.clone())
        {
            self.open_path(path, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
            self.navigate(-1, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::Home) && !self.files.is_empty() {
            self.select(0, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::End) && !self.files.is_empty() {
            self.select(self.files.len() - 1, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::F11) || key(egui::Modifiers::NONE, egui::Key::F) {
            self.toggle_fullscreen(ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::T) {
            self.settings.filmstrip = !self.settings.filmstrip;
            self.scroll_to_current = true;
        }
        if key(egui::Modifiers::NONE, egui::Key::I) {
            self.settings.info = !self.settings.info;
        }
        if key(egui::Modifiers::SHIFT, egui::Key::R) {
            self.view.rotation = (self.view.rotation + 3) % 4;
        } else if key(egui::Modifiers::NONE, egui::Key::R) {
            self.view.rotation = (self.view.rotation + 1) % 4;
        }
        if key(egui::Modifiers::NONE, egui::Key::H) {
            self.view.flip_x = !self.view.flip_x;
        }
        if key(egui::Modifiers::NONE, egui::Key::V) {
            self.view.flip_y = !self.view.flip_y;
        }
        if key(egui::Modifiers::NONE, egui::Key::Num0) {
            self.view.reset();
        }
        if key(egui::Modifiers::NONE, egui::Key::Num1) {
            self.actual_pixels(ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::Plus)
            || key(egui::Modifiers::NONE, egui::Key::Equals)
        {
            self.view
                .zoom_at(1.25, self.canvas_rect.center(), self.canvas_rect.center());
        }
        if key(egui::Modifiers::NONE, egui::Key::Minus) {
            self.view
                .zoom_at(0.8, self.canvas_rect.center(), self.canvas_rect.center());
        }
        if key(egui::Modifiers::NONE, egui::Key::Space) {
            self.toggle_play();
        }
        if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::C) {
            if let Some(entry) = self.current() {
                ctx.copy_text(entry.path.to_string_lossy().into_owned());
                self.notify("Path copied", false);
            }
        } else if key(egui::Modifiers::CTRL, egui::Key::C) {
            self.copy_image();
        }
        if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S) {
            self.export();
        }
        if key(egui::Modifiers::NONE, egui::Key::F2) {
            self.rename_dialog();
        }
        if key(egui::Modifiers::NONE, egui::Key::Delete) {
            self.trash_dialog();
        }
    }
    fn actual_pixels(&mut self, ctx: &egui::Context) {
        self.view.fit = false;
        self.view.zoom = 1.0 / ctx.pixels_per_point();
        self.view.pan = Vec2::ZERO;
    }
}

impl Flowa {
    fn toolbar(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        egui::Panel::top("toolbar")
            .exact_size(58.)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .inner_margin(egui::Margin::symmetric(14, 9)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    icons::logo(ui, 26., self.colors.accent);
                    ui.add_space(4.);
                    ui.menu_button("File", |ui| {
                        if ui.button("Open image…                    Ctrl+O").clicked() {
                            ui.close();
                            self.pick(false);
                        }
                        if ui.button("Open folder…             Ctrl+Shift+O").clicked() {
                            ui.close();
                            self.pick(true);
                        }
                        if ui
                            .add_enabled(
                                self.folder.is_some(),
                                egui::Button::new("Refresh folder                              F5"),
                            )
                            .clicked()
                        {
                            if let Some(path) = self
                                .current()
                                .map(|e| e.path.clone())
                                .or_else(|| self.folder.clone())
                            {
                                self.open_path(path, ctx);
                            }
                            ui.close();
                        }
                        ui.separator();
                        ui.add_enabled_ui(self.decoded.is_some() && self.busy.is_none(), |ui| {
                            if ui
                                .button("Copy image                         Ctrl+C")
                                .clicked()
                            {
                                ui.close();
                                self.copy_image();
                            }
                            if ui
                                .button("Export a copy…             Ctrl+Shift+S")
                                .clicked()
                            {
                                ui.close();
                                self.export();
                            }
                        });
                        ui.add_enabled_ui(self.current().is_some() && self.busy.is_none(), |ui| {
                            if ui
                                .button("Rename…                                F2")
                                .clicked()
                            {
                                ui.close();
                                self.rename_dialog();
                            }
                            if ui
                                .button("Move to trash…                     Delete")
                                .clicked()
                            {
                                ui.close();
                                self.trash_dialog();
                            }
                            if ui.button("Show folder in file manager").clicked()
                                && let Some(entry) = self.current()
                            {
                                self.actions.request(Task::Reveal(entry.path.clone()));
                                ui.close();
                            }
                        });
                        ui.separator();
                        if ui
                            .button("Preferences…                     Ctrl+,")
                            .clicked()
                        {
                            self.preferences_open = true;
                            ui.close();
                        }
                        if ui
                            .button("Quit                                  Ctrl+Q")
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    if icons::button(ui, Icon::Open, "Open image · Ctrl+O", false).clicked() {
                        self.pick(false);
                    }
                    ui.separator();
                    ui.add_enabled_ui(self.files.len() > 1, |ui| {
                        if icons::button(ui, Icon::Left, "Previous image · Left arrow", false)
                            .clicked()
                        {
                            self.navigate(-1, ctx);
                        }
                        if icons::button(ui, Icon::Right, "Next image · Right arrow", false)
                            .clicked()
                        {
                            self.navigate(1, ctx);
                        }
                    });
                    let narrow = ui.available_width() < 760.;
                    if !narrow {
                        let title = self.current().map(|e| e.name.as_str()).unwrap_or("flowa");
                        let width = (ui.available_width() - 480.0).clamp(40.0, 260.0);
                        ui.add_sized(
                            [width, 34.],
                            egui::Label::new(RichText::new(title).size(15.)).truncate(),
                        )
                        .on_hover_text(
                            self.current()
                                .map(|e| e.path.display().to_string())
                                .unwrap_or_else(|| "An open space for your images".into()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Fullscreen, "Fullscreen · F11", self.fullscreen)
                            .clicked()
                        {
                            self.toggle_fullscreen(ctx);
                        }
                        if icons::button(
                            ui,
                            Icon::Settings,
                            "Preferences · Ctrl+,",
                            self.preferences_open,
                        )
                        .clicked()
                        {
                            self.preferences_open = !self.preferences_open;
                        }
                        if icons::button(ui, Icon::Info, "Image details · I", self.settings.info)
                            .clicked()
                        {
                            self.settings.info = !self.settings.info;
                        }
                        if icons::button(
                            ui,
                            Icon::Filmstrip,
                            "Thumbnail strip · T",
                            self.settings.filmstrip,
                        )
                        .clicked()
                        {
                            self.settings.filmstrip = !self.settings.filmstrip;
                            self.scroll_to_current = true;
                        }
                        ui.separator();
                        ui.add_enabled_ui(self.decoded.is_some(), |ui| {
                            let animated =
                                self.decoded.as_ref().is_some_and(|d| d.frames.len() > 1);
                            if animated || self.slideshow {
                                let playing = if animated {
                                    self.playing
                                } else {
                                    self.slideshow
                                };
                                if icons::button(
                                    ui,
                                    if playing { Icon::Pause } else { Icon::Play },
                                    if playing {
                                        "Pause · Space"
                                    } else {
                                        "Play · Space"
                                    },
                                    false,
                                )
                                .clicked()
                                {
                                    self.toggle_play();
                                }
                            }
                            if !narrow
                                && icons::button(ui, Icon::Rotate, "Rotate clockwise · R", false)
                                    .clicked()
                            {
                                self.view.rotation = (self.view.rotation + 1) % 4;
                            }
                            if icons::button(ui, Icon::Fit, "Fit image · 0", self.view.fit)
                                .clicked()
                            {
                                self.view.reset();
                            }
                            if !narrow
                                && icons::button(
                                    ui,
                                    Icon::Actual,
                                    "Actual pixels · 1",
                                    !self.view.fit
                                        && (self.view.zoom * ctx.pixels_per_point() - 1.).abs()
                                            < 0.001,
                                )
                                .clicked()
                            {
                                self.actual_pixels(ctx);
                            }
                            if icons::button(ui, Icon::Plus, "Zoom in · +", false).clicked() {
                                self.view.zoom_at(
                                    1.25,
                                    self.canvas_rect.center(),
                                    self.canvas_rect.center(),
                                );
                            }
                            let percentage =
                                format!("{:.0}%", self.view.zoom * ctx.pixels_per_point() * 100.);
                            if ui
                                .add_sized(
                                    [62., 32.],
                                    egui::Button::new(RichText::new(percentage).size(13.))
                                        .frame(false),
                                )
                                .on_hover_text("Click for actual pixels")
                                .clicked()
                            {
                                self.actual_pixels(ctx);
                            }
                            if icons::button(ui, Icon::Minus, "Zoom out · -", false).clicked() {
                                self.view.zoom_at(
                                    0.8,
                                    self.canvas_rect.center(),
                                    self.canvas_rect.center(),
                                );
                            }
                        });
                    });
                });
            });
    }
    fn status(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(32.)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .inner_margin(egui::Margin::symmetric(18, 4)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(decoded) = &self.decoded {
                        ui.label(
                            RichText::new(format!("{}  ×  {}", decoded.size[0], decoded.size[1]))
                                .size(12.)
                                .color(self.colors.muted),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(&decoded.format)
                                .size(12.)
                                .color(self.colors.muted),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(catalog::human_size(decoded.file_size))
                                .size(12.)
                                .color(self.colors.muted),
                        );
                        if decoded.frames.len() > 1 {
                            ui.separator();
                            ui.label(
                                RichText::new(format!("{} frames", decoded.frames.len()))
                                    .size(12.)
                                    .color(self.colors.muted),
                            );
                        }
                    } else {
                        ui.label(
                            RichText::new("Drop an image or folder to begin")
                                .size(12.)
                                .color(self.colors.muted),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !self.files.is_empty() {
                            ui.label(
                                RichText::new(format!("{} / {}", self.index + 1, self.files.len()))
                                    .size(12.)
                                    .color(self.colors.muted),
                            );
                        }
                        if let Some(busy) = &self.busy {
                            ui.spinner();
                            ui.label(RichText::new(busy).size(12.).color(self.colors.muted));
                        } else if self.slideshow {
                            ui.label(
                                RichText::new("Slideshow")
                                    .size(12.)
                                    .color(self.colors.accent),
                            );
                        }
                    });
                });
            });
    }
    fn filmstrip(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        if self.files.is_empty() {
            return;
        }
        egui::Panel::bottom("filmstrip")
            .exact_size(116.)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(root, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("thumbnails")
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, viewport| {
                        let origin = ui.cursor().min;
                        let stride = 116.;
                        ui.set_min_size(vec2(self.files.len() as f32 * stride, 86.));
                        if self.scroll_to_current {
                            let selected = Rect::from_min_size(
                                origin + vec2(self.index as f32 * stride, 0.),
                                vec2(108., 86.),
                            );
                            ui.scroll_to_rect(selected, Some(egui::Align::Center));
                            self.scroll_to_current = false;
                        }
                        let start = (viewport.min.x / stride).floor().max(0.) as usize;
                        let end =
                            ((viewport.max.x / stride).ceil() as usize + 1).min(self.files.len());
                        let mut clicked = None;
                        for i in start..end {
                            let entry = &self.files[i];
                            let path = entry.path.clone();
                            let rect = Rect::from_min_size(
                                origin + vec2(i as f32 * stride, 0.),
                                vec2(108., 86.),
                            );
                            let response = ui.interact(rect, ui.scope_id().with(i), Sense::click());
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Button, true, &entry.name)
                            });
                            let selected = i == self.index;
                            ui.painter().rect_filled(
                                rect,
                                7.,
                                if selected {
                                    self.colors.raised
                                } else {
                                    self.colors.canvas
                                },
                            );
                            if let Some(Some(texture)) = self.thumbnails.get(&path) {
                                let area = Rect::from_min_max(
                                    rect.min + vec2(5., 4.),
                                    rect.max - vec2(5., 22.),
                                );
                                let size = texture.size_vec2();
                                let scale = (area.width() / size.x).min(area.height() / size.y);
                                ui.painter().image(
                                    texture.id(),
                                    Rect::from_center_size(area.center(), size * scale),
                                    Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                                    Color32::WHITE,
                                );
                            } else {
                                icons::paint(
                                    ui.painter(),
                                    Icon::Image,
                                    rect.center() - vec2(0., 9.),
                                    22.,
                                    self.colors.muted,
                                );
                                if !self.thumbnails.contains_key(&path)
                                    && !self.thumb_pending.contains(&path)
                                    && self
                                        .thumbnail_worker
                                        .requests
                                        .try_send(ThumbnailRequest {
                                            path: path.clone(),
                                            generation: self.scan_serial,
                                        })
                                        .is_ok()
                                {
                                    self.thumb_pending.insert(path);
                                }
                            }
                            let text_rect = Rect::from_min_max(
                                pos2(rect.left() + 5., rect.bottom() - 19.),
                                rect.max - vec2(5., 1.),
                            );
                            let text = egui::WidgetText::from(
                                RichText::new(&entry.name).size(11.).color(if selected {
                                    self.colors.text
                                } else {
                                    self.colors.muted
                                }),
                            );
                            let galley = text.into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                text_rect.width(),
                                egui::TextStyle::Small,
                            );
                            ui.painter()
                                .galley(text_rect.left_top(), galley, self.colors.text);
                            if selected || response.has_focus() || response.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    7.,
                                    Stroke::new(
                                        if selected { 1.5 } else { 1. },
                                        if selected || response.has_focus() {
                                            self.colors.accent
                                        } else {
                                            self.colors.border
                                        },
                                    ),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if response.clicked() {
                                clicked = Some(i);
                            }
                            response.on_hover_text(&entry.name);
                        }
                        if let Some(index) = clicked {
                            self.select(index, ctx);
                        }
                    });
            });
    }
    fn inspector(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        egui::Panel::right("inspector")
            .default_size(280.)
            .size_range(230.0..=420.0)
            .frame(egui::Frame::new().fill(self.colors.panel).inner_margin(20))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Image details");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Close, "Close image details", false).clicked() {
                            self.settings.info = false;
                        }
                    });
                });
                ui.add_space(18.);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(entry) = self.current() {
                        detail(ui, "FILE", &entry.name, self.colors);
                        detail(
                            ui,
                            "FOLDER",
                            &entry
                                .path
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default(),
                            self.colors,
                        );
                    }
                    if let Some(decoded) = &self.decoded {
                        detail(
                            ui,
                            "DIMENSIONS",
                            &format!("{} × {} pixels", decoded.size[0], decoded.size[1]),
                            self.colors,
                        );
                        detail(ui, "FORMAT", &decoded.format, self.colors);
                        detail(
                            ui,
                            "FILE SIZE",
                            &catalog::human_size(decoded.file_size),
                            self.colors,
                        );
                        detail(ui, "COLOR", &decoded.color, self.colors);
                        if decoded.frames.len() > 1 {
                            detail(
                                ui,
                                "ANIMATION",
                                &format!(
                                    "{} frames · {:.2} seconds",
                                    decoded.frames.len(),
                                    decoded.duration().as_secs_f64()
                                ),
                                self.colors,
                            );
                        }
                        for (key, value) in &decoded.metadata {
                            detail(ui, key, value, self.colors);
                        }
                    }
                    ui.add_space(10.);
                    if let Some(path) = self.current().map(|e| e.path.clone()) {
                        if ui.button("Copy file path").clicked() {
                            ctx.copy_text(path.to_string_lossy().into_owned());
                            self.notify("Path copied", false);
                        }
                        if ui.button("Open containing folder").clicked() {
                            self.actions.request(Task::Reveal(path));
                        }
                    }
                    ui.add_space(20.);
                    ui.label(
                        RichText::new(
                            "Rotation and flips are temporary. Export a copy to save what you see.",
                        )
                        .size(13.)
                        .color(self.colors.muted),
                    );
                });
            });
    }
    fn canvas_color(&self) -> Color32 {
        match self.settings.canvas {
            Canvas::Theme => self.colors.canvas,
            Canvas::Dark => Color32::from_rgb(24, 24, 26),
            Canvas::Black => Color32::BLACK,
            Canvas::White => Color32::WHITE,
            Canvas::Custom => Color32::from_rgb(
                self.settings.canvas_color[0],
                self.settings.canvas_color[1],
                self.settings.canvas_color[2],
            ),
        }
    }
    fn canvas(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        let base = self.canvas_color();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(base))
            .show(root, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                self.canvas_rect = rect;
                if self.decoded.is_some() {
                    if response.dragged() {
                        self.view.pan += response.drag_delta();
                        self.view.fit = false;
                    }
                    if response.hovered() {
                        let (scroll, pinch, pointer) = ctx.input(|i| {
                            (
                                i.smooth_scroll_delta.y,
                                i.zoom_delta(),
                                i.pointer.hover_pos().unwrap_or(rect.center()),
                            )
                        });
                        if (pinch - 1.).abs() > 0.0001 {
                            self.view.zoom_at(pinch, pointer, rect.center());
                        } else if scroll.abs() > 0.01 {
                            self.view
                                .zoom_at((scroll * 0.0025).exp(), pointer, rect.center());
                        }
                        if response.dragged() {
                            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                        } else {
                            ctx.set_cursor_icon(egui::CursorIcon::Grab);
                        }
                    }
                    if response.double_clicked() {
                        self.view.reset();
                    }
                    let size = self
                        .decoded
                        .as_ref()
                        .map(|d| vec2(d.size[0] as f32, d.size[1] as f32))
                        .unwrap_or(Vec2::splat(1.));
                    if self.view.fit {
                        self.view.zoom =
                            self.view
                                .fit_zoom(rect.size(), size, self.settings.fit_small);
                        if !self.settings.fit_small {
                            self.view.zoom = self.view.zoom.min(1.0 / ctx.pixels_per_point());
                        }
                        self.view.pan = Vec2::ZERO;
                    }
                    let image_rect = self.view.image_rect(size, rect);
                    let painter = ui.painter().with_clip_rect(rect);
                    self.paint_backdrop(&painter, image_rect, base);
                    if let Some(texture) = &self.preview_texture {
                        self.paint_source(
                            &painter,
                            texture.id(),
                            Rect::from_min_size(Pos2::ZERO, size),
                            size,
                            rect,
                        );
                    }
                    self.paint_tiles(&painter, ctx, size, rect, base);
                    response.context_menu(|ui| {
                        self.context_menu(ui, ctx);
                    });
                } else if self.loading {
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_center_size(rect.center(), vec2(250., 90.))),
                        |ui| {
                            ui.vertical_centered(|ui| {
                                ui.spinner();
                                ui.add_space(12.);
                                ui.label(RichText::new("Opening image…").color(self.colors.muted));
                            });
                        },
                    );
                } else if let Some(error) = self.load_error.clone() {
                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(Rect::from_center_size(
                            rect.center(),
                            vec2(rect.width().min(560.) - 40., 240.),
                        )),
                        |ui| {
                            ui.vertical_centered(|ui| {
                                icons::logo(ui, 40., self.colors.muted);
                                ui.add_space(14.);
                                ui.heading("This image could not be opened");
                                ui.add_space(8.);
                                ui.label(RichText::new(error).color(self.colors.muted));
                                ui.add_space(16.);
                                ui.horizontal(|ui| {
                                    if ui.button("Open another image").clicked() {
                                        self.pick(false);
                                    }
                                    if self.files.len() > 1 && ui.button("Next image").clicked() {
                                        self.navigate(1, ctx);
                                    }
                                });
                            });
                        },
                    );
                } else {
                    self.welcome(ui, rect);
                }
                if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
                    painter_drop(ui, rect, self.colors);
                }
            });
    }
    fn paint_backdrop(&self, painter: &egui::Painter, image_rect: Rect, base: Color32) {
        if !self.settings.checkerboard {
            return;
        }
        let visible = image_rect.intersect(painter.clip_rect());
        if !visible.is_positive() {
            return;
        }
        let square = self.settings.checker_size;
        let dark = u32::from(base.r()) + u32::from(base.g()) + u32::from(base.b()) < 384;
        let alternate: Color32 = egui::lerp(
            egui::Rgba::from(base)..=egui::Rgba::from(if dark {
                Color32::WHITE
            } else {
                Color32::BLACK
            }),
            self.settings.checker_contrast,
        )
        .into();
        let x0 = ((visible.left() - image_rect.left()) / square).floor() as i32;
        let y0 = ((visible.top() - image_rect.top()) / square).floor() as i32;
        let x1 = ((visible.right() - image_rect.left()) / square).ceil() as i32;
        let y1 = ((visible.bottom() - image_rect.top()) / square).ceil() as i32;
        let painter = painter.with_clip_rect(visible);
        for y in y0..y1 {
            for x in x0..x1 {
                if (x + y) % 2 == 0 {
                    painter.rect_filled(
                        Rect::from_min_size(
                            image_rect.min + vec2(x as f32 * square, y as f32 * square),
                            Vec2::splat(square),
                        ),
                        0.,
                        alternate,
                    );
                }
            }
        }
    }
    fn paint_source(
        &self,
        painter: &egui::Painter,
        texture: egui::TextureId,
        source: Rect,
        size: Vec2,
        canvas: Rect,
    ) {
        let points = [
            source.left_top(),
            source.right_top(),
            source.right_bottom(),
            source.left_bottom(),
        ]
        .map(|p| self.view.image_to_screen(p.to_vec2(), size, canvas));
        icons::uv_quad(painter, texture, points);
    }
    fn paint_tiles(
        &mut self,
        painter: &egui::Painter,
        ctx: &egui::Context,
        size: Vec2,
        canvas: Rect,
        base: Color32,
    ) {
        let Some(decoded) = self.decoded.as_ref() else {
            return;
        };
        if decoded.frames.len() > 1 && self.playing {
            return;
        }
        let Some(frame) = decoded.frames.get(self.frame_index) else {
            return;
        };
        if frame.preview.size == [decoded.size[0] as usize, decoded.size[1] as usize] {
            return;
        }
        let scale = self.view.zoom * ctx.pixels_per_point();
        let preview_scale = frame.preview.size[0] as f32 / size.x;
        if scale <= preview_scale * 1.02 {
            return;
        }
        let level = (1.0 / scale).log2().floor().clamp(0.0, 7.0) as u8;
        let span = loader::TILE_SIZE * (1_u32 << level);
        let visible = self.view.visible_source(size, canvas);
        if !visible.is_positive() {
            return;
        }
        let x0 = (visible.min.x.max(0.) as u32) / span;
        let y0 = (visible.min.y.max(0.) as u32) / span;
        let x1 = ((visible.max.x.ceil() as u32).div_ceil(span)).min(decoded.size[0].div_ceil(span));
        let y1 = ((visible.max.y.ceil() as u32).div_ceil(span)).min(decoded.size[1].div_ceil(span));
        let source_pixels = frame.pixels.clone();
        let mut keys = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                keys.push(TileKey {
                    generation: self.generation,
                    frame: self.frame_index,
                    level,
                    x,
                    y,
                });
            }
        }
        keys.sort_by_key(|key| {
            let dx = (key.x * span + span / 2) as f32 - visible.center().x;
            let dy = (key.y * span + span / 2) as f32 - visible.center().y;
            (dx * dx + dy * dy) as u64
        });
        let image_rect = self.view.image_rect(size, canvas);
        let mut requests = 0;
        for key in keys {
            if let Some(tile) = self.tiles.get_mut(&key) {
                tile.touched = self.paint_counter;
                let texture_id = tile.texture.id();
                let source = tile.source;
                let core = Rect::from_min_max(
                    pos2((key.x * span) as f32, (key.y * span) as f32),
                    pos2(
                        ((key.x + 1) * span).min(size.x as u32) as f32,
                        ((key.y + 1) * span).min(size.y as u32) as f32,
                    ),
                );
                let core_points = [
                    core.left_top(),
                    core.right_top(),
                    core.right_bottom(),
                    core.left_bottom(),
                ]
                .map(|p| self.view.image_to_screen(p.to_vec2(), size, canvas));
                let clip = Rect::from_points(&core_points).intersect(canvas);
                let painter = painter.with_clip_rect(clip);
                // Replace the preview underneath this tile, including its alpha contribution.
                painter.rect_filled(clip, 0., base);
                self.paint_backdrop(&painter, image_rect, base);
                self.paint_source(&painter, texture_id, source, size, canvas);
            } else if requests < 4
                && !self.pending_tiles.contains(&key)
                && self
                    .tile_worker
                    .requests
                    .try_send(TileRequest {
                        key,
                        pixels: source_pixels.clone(),
                    })
                    .is_ok()
            {
                self.pending_tiles.insert(key);
                requests += 1;
            }
        }
        if requests > 0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    fn context_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("Fit image                         0").clicked() {
            self.view.reset();
            ui.close();
        }
        if ui.button("Actual pixels                    1").clicked() {
            self.actual_pixels(ctx);
            ui.close();
        }
        if ui.button("Rotate clockwise              R").clicked() {
            self.view.rotation = (self.view.rotation + 1) % 4;
            ui.close();
        }
        if ui.button("Flip horizontally               H").clicked() {
            self.view.flip_x = !self.view.flip_x;
            ui.close();
        }
        if ui.button("Flip vertically                   V").clicked() {
            self.view.flip_y = !self.view.flip_y;
            ui.close();
        }
        ui.separator();
        if ui
            .button(if self.slideshow {
                "Stop slideshow"
            } else {
                "Start slideshow"
            })
            .clicked()
        {
            self.slideshow = !self.slideshow;
            self.next_slide =
                Instant::now() + Duration::from_secs_f32(self.settings.slideshow_seconds);
            ui.close();
        }
        if self.decoded.as_ref().is_some_and(|d| d.frames.len() > 1)
            && ui
                .button(if self.playing {
                    "Pause animation"
                } else {
                    "Play animation"
                })
                .clicked()
        {
            self.toggle_play();
            ui.close();
        }
        ui.separator();
        if ui.button("Copy image                  Ctrl+C").clicked() {
            self.copy_image();
            ui.close();
        }
        if ui.button("Export a copy…").clicked() {
            self.export();
            ui.close();
        }
        if ui.button("Rename…").clicked() {
            self.rename_dialog();
            ui.close();
        }
        if ui.button("Move to trash…").clicked() {
            self.trash_dialog();
            ui.close();
        }
        ui.separator();
        ui.menu_button("Sort folder", |ui| {
            let old = (self.settings.sort, self.settings.reverse_sort);
            for sort in [SortOrder::Name, SortOrder::Modified, SortOrder::Size] {
                ui.selectable_value(&mut self.settings.sort, sort, sort.name());
            }
            ui.separator();
            ui.checkbox(&mut self.settings.reverse_sort, "Reverse order");
            if old != (self.settings.sort, self.settings.reverse_sort) {
                self.sort();
            }
        });
    }
    fn welcome(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let width = rect.width().min(480.);
        let height = if self.settings.recent.is_empty() {
            285.
        } else {
            425.
        };
        let area = Rect::from_center_size(rect.center() - vec2(0., 10.), vec2(width, height));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            ui.vertical_centered(|ui| {
                icons::logo(ui, 58., self.colors.accent);
                ui.add_space(18.);
                ui.label(RichText::new("flowa").size(34.));
                ui.add_space(8.);
                ui.label(
                    RichText::new("An open space for your images.")
                        .size(16.)
                        .color(self.colors.muted),
                );
                ui.add_space(24.);
                ui.horizontal(|ui| {
                    ui.add_space((width - 308.).max(0.) / 2.);
                    if ui
                        .add_sized(
                            [146., 42.],
                            egui::Button::new("Open image").fill(self.colors.raised),
                        )
                        .clicked()
                    {
                        self.pick(false);
                    }
                    if ui
                        .add_sized([146., 42.], egui::Button::new("Open folder"))
                        .clicked()
                    {
                        self.pick(true);
                    }
                });
                ui.add_space(14.);
                ui.label(
                    RichText::new("or drop an image or folder anywhere")
                        .size(13.)
                        .color(self.colors.muted),
                );
                if !self.settings.recent.is_empty() {
                    ui.add_space(30.);
                    ui.label(
                        RichText::new("RECENT IMAGES")
                            .size(11.)
                            .color(self.colors.muted),
                    );
                    ui.add_space(8.);
                    let mut open = None;
                    for path in self.settings.recent.iter().take(3) {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        if ui
                            .add_sized([width.min(360.), 30.], egui::Button::new(name).frame(false))
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            open = Some(path.clone());
                        }
                    }
                    if let Some(path) = open {
                        self.open_path(path, ui.ctx());
                    }
                }
            });
        });
    }
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str, c: Colors) {
    ui.label(RichText::new(label).size(11.).color(c.muted));
    ui.add(egui::Label::new(value).wrap());
    ui.add_space(15.);
}
fn painter_drop(ui: &egui::Ui, rect: Rect, c: Colors) {
    ui.painter()
        .rect_filled(rect, 0., c.canvas.gamma_multiply(0.94));
    ui.painter().rect_stroke(
        rect.shrink(16.),
        12.,
        Stroke::new(2., c.accent),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to open",
        egui::FontId::proportional(26.),
        c.text,
    );
}

impl Flowa {
    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        let response = egui::Modal::new(egui::Id::unique("file-dialog")).show(ctx, |ui| {
            ui.set_width(400.);
            match &mut dialog {
                Dialog::Rename { path, name } => {
                    ui.heading("Rename image");
                    ui.add_space(12.);
                    ui.label("New filename");
                    let id = ui.scope_id().with("rename-filename");
                    let submit = ui.memory(|m| m.has_focus(id))
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let edit = ui.add(
                        egui::TextEdit::singleline(name)
                            .id(id)
                            .desired_width(f32::INFINITY),
                    );
                    if ui.memory(|m| m.focused().is_none()) {
                        edit.request_focus();
                    }
                    let valid = catalog::valid_filename(name);
                    if !valid {
                        ui.label(
                            RichText::new(
                                "Use a filename without path separators or reserved characters.",
                            )
                            .color(self.colors.muted),
                        );
                    }
                    ui.add_space(16.);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                        if ui.add_enabled(valid, egui::Button::new("Rename")).clicked()
                            || (valid && submit)
                        {
                            self.actions.request(Task::Rename {
                                source: path.clone(),
                                name: name.clone(),
                            });
                            self.busy = Some("Renaming image…".into());
                            keep = false;
                        }
                    });
                }
                Dialog::Trash(path) => {
                    ui.heading("Move this image to trash?");
                    ui.add_space(12.);
                    ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                    ui.add_space(8.);
                    ui.label(
                        RichText::new("You can restore it from your system’s trash.")
                            .color(self.colors.muted),
                    );
                    ui.add_space(18.);
                    ui.horizontal(|ui| {
                        if ui.button("Keep image").clicked() {
                            keep = false;
                        }
                        if ui.button("Move to trash").clicked() {
                            self.actions.request(Task::Trash(path.clone()));
                            self.busy = Some("Moving to trash…".into());
                            keep = false;
                        }
                    });
                }
            }
        });
        if response.should_close() {
            keep = false;
        }
        if keep {
            self.dialog = Some(dialog);
        }
    }
    fn notice(&mut self, ctx: &egui::Context) {
        let Some(notice) = self.notice.as_ref() else {
            return;
        };
        if Instant::now() >= notice.until {
            self.notice = None;
            return;
        }
        let message = notice.message.clone();
        let error = notice.error;
        egui::Area::new(egui::Id::unique("notice"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0., -48.))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .stroke(Stroke::new(
                        1.,
                        if error {
                            Color32::from_rgb(230, 125, 125)
                        } else {
                            self.colors.border
                        },
                    ))
                    .corner_radius(10)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if error {
                                ui.label(
                                    RichText::new("!").color(Color32::from_rgb(230, 125, 125)),
                                );
                            }
                            ui.add(egui::Label::new(message).wrap().selectable(true))
                                .on_hover_text("Notification");
                            if icons::button(ui, Icon::Close, "Dismiss notification", false)
                                .clicked()
                            {
                                self.notice = None;
                            }
                        });
                    });
            });
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    fn screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot_requested
            || self.started.elapsed() < Duration::from_secs(2)
            || self.loading
        {
            return;
        }
        let Some(path) = self.launch.screenshot.clone() else {
            if self.launch.smoke_test && self.started.elapsed() > Duration::from_secs(3) {
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
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
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
                    texture.set(decoded.frames[self.frame_index].preview.clone(), options);
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
