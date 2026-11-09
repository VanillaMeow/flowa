//! Visible thumbnails and horizontal folder navigation

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use egui::{
    Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, pos2,
    vec2,
};

use crate::{
    app::Command,
    services::{
        thumbnails::{ThumbnailRequest, ThumbnailWorker},
        worker::Generation,
    },
    viewer::{browser::Browser, catalog::Entry},
};

use super::{
    icons::{self, Icon},
    style::{margin, radius, text},
    theme::Colors,
};

const THUMBNAIL_UPLOADS_PER_FRAME: usize = 4;
const MAX_CACHED_THUMBNAILS: usize = 128;
const ITEM_STRIDE: f32 = 116.;
const ITEM_WIDTH: f32 = 108.;
const ITEM_HEIGHT: f32 = 86.;
/// Items, panel margins, and room for the horizontal scroll bar
const STRIP_HEIGHT: f32 = 116.;

pub(crate) struct Filmstrip {
    worker: ThumbnailWorker,
    generation: Generation,
    /// The image memory limit in bytes, applied to thumbnail decodes
    budget: u64,
    textures: HashMap<PathBuf, Option<TextureHandle>>,
    pending: HashSet<PathBuf>,
    order: Vec<PathBuf>,
    scroll_to_current: bool,
}

impl Filmstrip {
    pub fn new(ctx: egui::Context, budget: u64) -> Self {
        let generation = Generation::default();

        Self {
            worker: ThumbnailWorker::new(ctx, &generation),
            generation,
            budget,
            textures: HashMap::new(),
            pending: HashSet::new(),
            order: Vec::new(),
            scroll_to_current: false,
        }
    }

    pub fn reset(&mut self, generation: u64) {
        self.generation.set(generation);
        self.textures.clear();
        self.pending.clear();
        self.order.clear();
    }

    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
    }

    pub fn remove(&mut self, path: &Path) {
        self.textures.remove(path);
        self.order.retain(|cached| cached != path);
    }

    pub fn reveal_selection(&mut self) {
        self.scroll_to_current = true;
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        for _ in 0..THUMBNAIL_UPLOADS_PER_FRAME {
            let Ok(result) = self.worker.results.try_recv() else {
                break;
            };

            if !self.generation.is_current(result.generation) {
                continue;
            }

            self.pending.remove(&result.path);
            let texture = result.image.map(|image| {
                ctx.load_texture(
                    format!("thumbnail:{}", result.path.display()),
                    image,
                    TextureOptions::LINEAR,
                )
            });

            self.cache(result.path, texture);
        }
    }

    pub fn cache(&mut self, path: PathBuf, texture: Option<TextureHandle>) {
        self.order.retain(|p| *p != path);
        self.order.push(path.clone());

        // A cached None also prevents retrying a corrupt thumbnail every frame
        self.textures.insert(path, texture);

        while self.order.len() > MAX_CACHED_THUMBNAILS {
            let old = self.order.remove(0);
            self.textures.remove(&old);
        }
    }

    pub fn show(
        &mut self,
        root: &mut egui::Ui,
        browser: &Browser,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        if browser.entries().is_empty() {
            return;
        }

        let frame = egui::Frame::new()
            .fill(colors.panel)
            .inner_margin(margin::BOTTOM_PANEL);

        egui::Panel::bottom("filmstrip")
            .exact_size(STRIP_HEIGHT)
            .frame(frame)
            .show(root, |ui| {
                self.show_scroll_area(ui, browser, colors, commands)
            });
    }

    fn show_scroll_area(
        &mut self,
        ui: &mut egui::Ui,
        browser: &Browser,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        ui.style_mut().always_scroll_the_only_direction = true;

        egui::ScrollArea::horizontal()
            .id_salt("thumbnails")
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                self.show_visible_items(ui, viewport, browser, colors, commands);
            });
    }

    fn show_visible_items(
        &mut self,
        ui: &mut egui::Ui,
        viewport: Rect,
        browser: &Browser,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        let origin = ui.cursor().min;
        let entries = browser.entries();

        ui.set_min_size(vec2(entries.len() as f32 * ITEM_STRIDE, ITEM_HEIGHT));

        if self.scroll_to_current {
            let selected = item_rect(origin, browser.index());
            ui.scroll_to_rect(selected, Some(egui::Align::Center));
            self.scroll_to_current = false;
        }

        // Large folders only create widgets for thumbnails in view
        let start = (viewport.min.x / ITEM_STRIDE).floor().max(0.) as usize;
        let end =
            ((viewport.max.x / ITEM_STRIDE).ceil() as usize + 1).min(entries.len());
        let mut clicked = None;

        for (index, entry) in entries.iter().enumerate().take(end).skip(start) {
            let selected = index == browser.index();
            let rect = item_rect(origin, index);

            let response = self.show_item(ui, entry, index, rect, selected, colors);

            if response.clicked() {
                clicked = Some(index);
            } else if response.hovered() && !selected {
                commands.push(Command::PreloadImage(index));
            }
        }

        // Apply navigation after the panel releases its borrowed browser state
        if let Some(index) = clicked {
            commands.push(Command::SelectImage(index));
        }
    }

    fn show_item(
        &mut self,
        ui: &mut egui::Ui,
        entry: &Entry,
        index: usize,
        rect: Rect,
        selected: bool,
        colors: Colors,
    ) -> egui::Response {
        let response = ui.interact(rect, ui.scope_id().with(index), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, &entry.name)
        });

        let background = if selected {
            colors.raised
        } else {
            colors.canvas
        };
        ui.painter().rect_filled(rect, radius::CONTROL, background);

        self.paint_thumbnail(ui.painter(), &entry.path, rect, colors);
        paint_caption(ui, &entry.name, rect, selected, colors);
        paint_outline(ui.painter(), &response, selected, colors);

        response.on_hover_text(&entry.name)
    }

    fn paint_thumbnail(
        &mut self,
        painter: &egui::Painter,
        path: &Path,
        rect: Rect,
        colors: Colors,
    ) {
        let Some(Some(texture)) = self.textures.get(path) else {
            icons::paint(
                painter,
                if crate::viewer::catalog::is_video(path) {
                    Icon::Play
                } else {
                    Icon::Image
                },
                rect.center() - vec2(0., 9.),
                22.,
                colors.muted,
            );
            self.request_thumbnail(path);
            return;
        };

        let area =
            Rect::from_min_max(rect.min + vec2(5., 4.), rect.max - vec2(5., 22.));
        let size = texture.size_vec2();
        let scale = (area.width() / size.x).min(area.height() / size.y);
        let destination = Rect::from_center_size(area.center(), size * scale);
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1., 1.));

        painter.image(texture.id(), destination, uv, Color32::WHITE);
    }

    fn request_thumbnail(&mut self, path: &Path) {
        if self.textures.contains_key(path) || self.pending.contains(path) {
            return;
        }

        let request = ThumbnailRequest {
            path: path.to_path_buf(),
            generation: self.generation.get(),
            budget: self.budget,
        };

        if self.worker.request(request) {
            self.pending.insert(path.to_path_buf());
        }
    }
}

fn item_rect(origin: Pos2, index: usize) -> Rect {
    Rect::from_min_size(
        origin + vec2(index as f32 * ITEM_STRIDE, 0.),
        vec2(ITEM_WIDTH, ITEM_HEIGHT),
    )
}

fn paint_caption(
    ui: &egui::Ui,
    name: &str,
    rect: Rect,
    selected: bool,
    colors: Colors,
) {
    let text_rect = Rect::from_min_max(
        pos2(rect.left() + 5., rect.bottom() - 19.),
        rect.max - vec2(5., 1.),
    );
    let color = if selected { colors.text } else { colors.muted };
    let text =
        egui::WidgetText::from(RichText::new(name).size(text::CAPTION).color(color));
    let galley = text.into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        text_rect.width(),
        egui::TextStyle::Small,
    );

    ui.painter()
        .galley(text_rect.left_top(), galley, colors.text);
}

fn paint_outline(
    painter: &egui::Painter,
    response: &egui::Response,
    selected: bool,
    colors: Colors,
) {
    if !selected && !response.has_focus() && !response.hovered() {
        return;
    }

    let width = if selected { 1.5 } else { 1. };
    let color = if selected || response.has_focus() {
        colors.accent
    } else {
        colors.border
    };

    painter.rect_stroke(
        response.rect,
        radius::CONTROL,
        Stroke::new(width, color),
        egui::StrokeKind::Inside,
    );
}
