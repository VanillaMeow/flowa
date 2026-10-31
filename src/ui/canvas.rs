//! Image interaction and the canvas's loading, error, and welcome states

use egui::{Rect, RichText, Sense, Stroke, Vec2, vec2};

use crate::{
    app::Command,
    settings::Settings,
    viewer::{
        image_session::{DecodedImage, ImageSession},
        transform::View,
    },
};

use super::{
    icons,
    rendering::image_renderer::{self as renderer, ImageRenderer, RenderFrame},
    theme::Colors,
};

pub(crate) struct CanvasContent<'a> {
    pub image: &'a ImageSession,
    pub settings: &'a Settings,
    pub colors: Colors,
    pub can_navigate: bool,
}

pub(crate) struct ImageCanvas {
    pub view: View,
    pub rect: Rect,
    pub renderer: ImageRenderer,
}

impl ImageCanvas {
    pub fn new(ctx: egui::Context, nearest: bool) -> Self {
        Self {
            view: View::default(),
            rect: Rect::NOTHING,
            renderer: ImageRenderer::new(ctx, nearest),
        }
    }

    pub fn reset(&mut self, generation: u64, remember_view: bool) {
        self.renderer.reset(generation);

        if !remember_view {
            self.view = View::default();
        }
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

        let context_menu = if let Some(decoded) = &content.image.decoded {
            self.show_image(ui, &response, content, decoded);
            Some(response)
        } else {
            show_empty_state(ui, rect, content, commands);
            None
        };

        if ui.input(|input| !input.raw.hovered_files.is_empty()) {
            paint_drop_target(ui, rect, content.colors);
        }

        context_menu
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

        self.handle_pointer(response, ctx);
        self.fit_image(size, content.settings, ctx.pixels_per_point());

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

    fn fit_image(&mut self, size: Vec2, settings: &Settings, pixels_per_point: f32) {
        if !self.view.fit {
            return;
        }

        self.view.zoom = self
            .view
            .fit_zoom(self.rect.size(), size, settings.fit_small);

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
    if content.image.loading {
        show_loading(ui, rect, content.colors);
    } else if let Some(error) = &content.image.error {
        show_error(ui, rect, error, content, commands);
    } else {
        welcome(ui, rect, content.settings, content.colors, commands);
    }
}

fn show_loading(ui: &mut egui::Ui, rect: Rect, colors: Colors) {
    let area = Rect::from_center_size(rect.center(), vec2(250., 90.));

    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        ui.vertical_centered(|ui| {
            ui.spinner();
            ui.add_space(12.);
            ui.label(RichText::new("Opening image…").color(colors.muted));
        });
    });
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
    ui.add_space(14.);
    ui.heading("This image could not be opened");
    ui.add_space(8.);
    ui.label(RichText::new(error).color(content.colors.muted));
    ui.add_space(16.);

    ui.horizontal(|ui| {
        if ui.button("Open another image").clicked() {
            commands.push(Command::OpenFile);
        }

        if content.can_navigate && ui.button("Next image").clicked() {
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
        285.
    } else {
        425.
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
    icons::logo(ui, 58., colors.accent);
    ui.add_space(18.);
    ui.label(RichText::new("flowa").size(34.));
    ui.add_space(8.);
    ui.label(
        RichText::new("An open space for your images.")
            .size(16.)
            .color(colors.muted),
    );
    ui.add_space(24.);

    open_buttons(ui, width, colors, commands);

    ui.add_space(14.);
    ui.label(
        RichText::new("or drop an image or folder anywhere")
            .size(13.)
            .color(colors.muted),
    );

    if !settings.recent.is_empty() {
        recent_images(ui, width, settings, colors, commands);
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

        let open_image = egui::Button::new("Open image").fill(colors.raised);

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

fn recent_images(
    ui: &mut egui::Ui,
    width: f32,
    settings: &Settings,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    ui.add_space(30.);
    ui.label(RichText::new("RECENT IMAGES").size(11.).color(colors.muted));
    ui.add_space(8.);

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
