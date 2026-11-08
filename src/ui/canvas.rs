//! Image interaction and the canvas's loading, error, and welcome states

use std::time::Duration;

use egui::{Rect, RichText, Sense, Stroke, Vec2, vec2};

use crate::{
    app::Command,
    settings::Settings,
    video::VideoSession,
    viewer::{
        image_session::{DecodedImage, ImageSession},
        transform::View,
    },
};

use super::{
    icons,
    rendering::image_renderer::{
        self as renderer, FrozenPreview, ImageRenderer, RenderFrame,
    },
    style::{space, text},
    theme::Colors,
    widgets,
};

const LOADING_DELAY: Duration = Duration::from_millis(150);
const LOADING_FADE: Duration = Duration::from_millis(100);

pub(crate) struct CanvasContent<'a> {
    pub image: &'a ImageSession,
    pub video: &'a VideoSession,
    pub settings: &'a Settings,
    pub colors: Colors,
    pub can_navigate: bool,
    pub fullscreen: bool,
}

pub(crate) struct ImageCanvas {
    pub view: View,
    pub rect: Rect,
    pub renderer: ImageRenderer,
    previous: Option<FrozenPreview>,
    displayed_size: Option<Vec2>,
}

impl ImageCanvas {
    pub fn new(ctx: egui::Context, nearest: bool) -> Self {
        Self {
            view: View::default(),
            rect: Rect::NOTHING,
            renderer: ImageRenderer::new(ctx, nearest),
            previous: None,
            displayed_size: None,
        }
    }

    pub fn reset(&mut self, generation: u64, remember_view: bool) {
        if let Some(size) = self.displayed_size.take()
            && let Some(preview) = self.renderer.freeze(size, &self.view)
        {
            self.previous = Some(preview);
        }

        self.renderer.reset(generation);

        if !remember_view {
            // Keep the last zoom until the next image's dimensions are available
            self.view = View {
                zoom: self.view.zoom,
                ..Default::default()
            };
        }
    }

    pub fn clear(&mut self, generation: u64) {
        self.previous = None;
        self.displayed_size = None;
        self.renderer.reset(generation);
    }

    pub fn actual_pixels(&mut self, ctx: &egui::Context) {
        // Egui measures in logical points, but 100% means one physical pixel
        self.view.fit = false;
        self.view.zoom = 1.0 / ctx.pixels_per_point();
        self.view.pan = Vec2::ZERO;
    }

    pub fn zoom(&mut self, factor: f32) {
        let center = self.rect.center();
        self.view.zoom_at(factor, center, center);
    }

    pub fn show(
        &mut self,
        root: &mut egui::Ui,
        content: CanvasContent<'_>,
        commands: &mut Vec<Command>,
    ) -> Option<egui::Response> {
        let background = renderer::canvas_color(content.settings, content.colors);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(background))
            .show(root, |ui| self.show_content(ui, &content, commands))
            .inner
    }

    fn show_content(
        &mut self,
        ui: &mut egui::Ui,
        content: &CanvasContent<'_>,
        commands: &mut Vec<Command>,
    ) -> Option<egui::Response> {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.rect = rect;

        // Only the loading state keeps painting the previous file's frozen preview
        let loading_image = !content.video.state.active
            && content.image.decoded.is_none()
            && content.image.loading;
        if !loading_image {
            self.previous = None;
        }

        let context_menu = if content.video.state.active {
            self.show_video(ui, &response, content, commands);
            Some(response)
        } else if let Some(decoded) = &content.image.decoded {
            self.show_image(ui, &response, content, decoded);
            Some(response)
        } else if loading_image {
            self.show_loading(ui, content);
            None
        } else {
            show_empty_state(ui, rect, content, commands);
            None
        };

        if ui.input(|input| !input.raw.hovered_files.is_empty()) {
            paint_drop_target(ui, rect, content.colors);
        }

        context_menu
    }

    fn show_video(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        content: &CanvasContent<'_>,
        commands: &mut Vec<Command>,
    ) {
        let video = content.video;
        if let Some(error) = &video.state.error {
            show_error(ui, self.rect, error, content, commands);
            return;
        }

        let Some(texture) = video.texture else {
            show_loading(ui, self.rect, content.colors, 1.);
            return;
        };

        let size = vec2(video.state.size[0] as f32, video.state.size[1] as f32);
        self.displayed_size = Some(size);
        self.handle_pointer(response, ui.ctx());
        self.fit_image(size, content, ui.ctx().pixels_per_point());

        let painter = ui.painter().with_clip_rect(self.rect);
        super::rendering::image_renderer::paint_source(
            &painter,
            texture,
            Rect::from_min_size(egui::Pos2::ZERO, size),
            size,
            self.rect,
            &self.view,
        );
    }

    fn show_image(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        content: &CanvasContent<'_>,
        decoded: &DecodedImage,
    ) {
        let ctx = ui.ctx();
        let size = vec2(decoded.size[0] as f32, decoded.size[1] as f32);
        self.displayed_size = Some(size);

        self.handle_pointer(response, ctx);
        self.fit_image(size, content, ctx.pixels_per_point());

        let painter = ui.painter().with_clip_rect(self.rect);
        let frame = RenderFrame {
            image: content.image,
            view: &self.view,
            canvas: self.rect,
            settings: content.settings,
            colors: content.colors,
        };

        self.renderer.paint(&painter, ctx, frame);
    }

    fn show_loading(&self, ui: &mut egui::Ui, content: &CanvasContent<'_>) {
        if let Some(preview) = &self.previous {
            let painter = ui.painter().with_clip_rect(self.rect);
            preview.paint(&painter, self.rect, content.settings, content.colors);
        }

        let elapsed = content.image.loading_age();

        if elapsed < LOADING_DELAY {
            ui.ctx().request_repaint_after(LOADING_DELAY - elapsed);
            return;
        }

        let opacity = ((elapsed - LOADING_DELAY).as_secs_f32()
            / LOADING_FADE.as_secs_f32())
        .min(1.);

        if opacity < 1. {
            ui.ctx().request_repaint();
        }

        show_loading(ui, self.rect, content.colors, opacity);
    }

    fn handle_pointer(&mut self, response: &egui::Response, ctx: &egui::Context) {
        if response.dragged() {
            self.view.pan += response.drag_delta();
            self.view.fit = false;
        }

        if response.hovered() {
            let (scroll, pinch, pointer) = ctx.input(|input| {
                (
                    input.smooth_scroll_delta.y,
                    input.zoom_delta(),
                    input.pointer.hover_pos().unwrap_or(self.rect.center()),
                )
            });

            if (pinch - 1.).abs() > 0.0001 {
                self.view.zoom_at(pinch, pointer, self.rect.center());
            } else if scroll.abs() > 0.01 {
                let factor = (scroll * 0.0025).exp();
                self.view.zoom_at(factor, pointer, self.rect.center());
            }

            let cursor = if response.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            };
            ctx.set_cursor_icon(cursor);
        }

        if response.double_clicked() {
            self.view.reset();
        }
    }

    fn fit_image(
        &mut self,
        size: Vec2,
        content: &CanvasContent<'_>,
        pixels_per_point: f32,
    ) {
        if !self.view.fit {
            return;
        }

        let padding = if content.fullscreen { 0. } else { 24. };
        let settings = content.settings;
        self.view.zoom =
            self.view
                .fit_zoom(self.rect.size(), size, settings.fit_small, padding);

        // Small images should not be enlarged on high-DPI displays
        if !settings.fit_small {
            self.view.zoom = self.view.zoom.min(1.0 / pixels_per_point);
        }

        self.view.pan = Vec2::ZERO;
    }
}

fn show_empty_state(
    ui: &mut egui::Ui,
    rect: Rect,
    content: &CanvasContent<'_>,
    commands: &mut Vec<Command>,
) {
    if let Some(error) = &content.image.error {
        show_error(ui, rect, error, content, commands);
    } else {
        welcome(ui, rect, content.settings, content.colors, commands);
    }
}

fn show_loading(ui: &mut egui::Ui, rect: Rect, colors: Colors, opacity: f32) {
    let area = Rect::from_center_size(rect.center(), vec2(250., 90.));

    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        ui.set_opacity(opacity);
        ui.vertical_centered(|ui| loading_contents(ui, colors));
    });
}

fn loading_contents(ui: &mut egui::Ui, colors: Colors) {
    ui.spinner();
    ui.add_space(space::MD);
    ui.label(RichText::new("Opening file…").color(colors.muted));
}

fn show_error(
    ui: &mut egui::Ui,
    rect: Rect,
    error: &str,
    content: &CanvasContent<'_>,
    commands: &mut Vec<Command>,
) {
    let size = vec2(rect.width().min(560.) - 40., 240.);
    let area = Rect::from_center_size(rect.center(), size);

    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        ui.vertical_centered(|ui| error_contents(ui, error, content, commands));
    });
}

fn error_contents(
    ui: &mut egui::Ui,
    error: &str,
    content: &CanvasContent<'_>,
    commands: &mut Vec<Command>,
) {
    icons::logo(ui, 40., content.colors.muted);
    ui.add_space(space::MD);
    ui.heading("This file could not be opened");
    ui.add_space(space::SM);
    ui.label(RichText::new(error).color(content.colors.muted));
    ui.add_space(space::LG);

    ui.horizontal(|ui| {
        if ui.button("Open another file").clicked() {
            commands.push(Command::OpenFile);
        }

        if content.can_navigate && ui.button("Next file").clicked() {
            commands.push(Command::Navigate(1));
        }
    });
}

fn welcome(
    ui: &mut egui::Ui,
    rect: Rect,
    settings: &Settings,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    let width = rect.width().min(480.);
    let height = if settings.recent.is_empty() {
        250.
    } else {
        390.
    };
    let center = rect.center() - vec2(0., 10.);
    let area = Rect::from_center_size(center, vec2(width, height));

    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        ui.vertical_centered(|ui| {
            welcome_contents(ui, width, settings, colors, commands)
        });
    });
}

fn welcome_contents(
    ui: &mut egui::Ui,
    width: f32,
    settings: &Settings,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    icons::logo(ui, 64., colors.accent);
    ui.add_space(space::LG);
    ui.label(RichText::new("flowa").size(text::DISPLAY));
    ui.add_space(space::XL);

    open_buttons(ui, width, colors, commands);

    ui.add_space(space::MD);
    widgets::hint(ui, "or drop an image, video, or folder anywhere", colors);

    if !settings.recent.is_empty() {
        recent_paths(ui, width, settings, colors, commands);
    }
}

fn open_buttons(
    ui: &mut egui::Ui,
    width: f32,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    ui.horizontal(|ui| {
        ui.add_space((width - 308.).max(0.) / 2.);

        let open_image = egui::Button::new("Open file").fill(colors.raised);

        if ui.add_sized([146., 42.], open_image).clicked() {
            commands.push(Command::OpenFile);
        }

        if ui
            .add_sized([146., 42.], egui::Button::new("Open folder"))
            .clicked()
        {
            commands.push(Command::OpenFolder);
        }
    });
}

fn recent_paths(
    ui: &mut egui::Ui,
    width: f32,
    settings: &Settings,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    ui.add_space(space::XL);
    widgets::caption(ui, "Recent", colors);
    ui.add_space(space::SM);

    let mut open = None;

    for path in settings.recent.iter().take(3) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let button = egui::Button::new(name).frame(false);
        let response = ui
            .add_sized([width.min(360.), 30.], button)
            .on_hover_text(path.display().to_string());

        if response.clicked() {
            open = Some(path.clone());
        }
    }

    if let Some(path) = open {
        commands.push(Command::OpenPath(path));
    }
}

fn paint_drop_target(ui: &egui::Ui, rect: Rect, colors: Colors) {
    let painter = ui.painter();

    painter.rect_filled(rect, 0., colors.canvas.gamma_multiply(0.94));
    painter.rect_stroke(
        rect.shrink(16.),
        12.,
        Stroke::new(2., colors.accent),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to open",
        egui::FontId::proportional(26.),
        colors.text,
    );
}
