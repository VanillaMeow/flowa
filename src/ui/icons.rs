//! Lucide control icons, reusable widgets, and the Flowa flower logo

use egui::{Color32, Pos2, Response, Sense, Ui, Vec2, vec2};

use super::{rendering::icon_cache, theme};

pub const LICENSE: &str = include_str!("../../assets/icons/lucide/LICENSE");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Open,
    Left,
    Right,
    Fit,
    Actual,
    Minus,
    Plus,
    Rotate,
    Filmstrip,
    Info,
    Settings,
    Fullscreen,
    Close,
    Play,
    Pause,
    Volume,
    Muted,
    Image,
    Check,
    Alert,
}

impl Icon {
    pub(crate) fn svg(self) -> &'static str {
        match self {
            Self::Open => include_str!("../../assets/icons/lucide/folder-open.svg"),
            Self::Left => include_str!("../../assets/icons/lucide/chevron-left.svg"),
            Self::Right => include_str!("../../assets/icons/lucide/chevron-right.svg"),
            Self::Fit => include_str!("../../assets/icons/lucide/scan.svg"),
            Self::Actual => include_str!("../../assets/icons/lucide/ratio.svg"),
            Self::Minus => include_str!("../../assets/icons/lucide/minus.svg"),
            Self::Plus => include_str!("../../assets/icons/lucide/plus.svg"),
            Self::Rotate => include_str!("../../assets/icons/lucide/rotate-cw.svg"),
            Self::Filmstrip => {
                include_str!("../../assets/icons/lucide/gallery-thumbnails.svg")
            }
            Self::Info => include_str!("../../assets/icons/lucide/info.svg"),
            Self::Settings => include_str!("../../assets/icons/lucide/settings.svg"),
            Self::Fullscreen => include_str!("../../assets/icons/lucide/maximize.svg"),
            Self::Close => include_str!("../../assets/icons/lucide/x.svg"),
            Self::Play => include_str!("../../assets/icons/lucide/play.svg"),
            Self::Pause => include_str!("../../assets/icons/lucide/pause.svg"),
            Self::Volume => include_str!("../../assets/icons/lucide/volume-2.svg"),
            Self::Muted => include_str!("../../assets/icons/lucide/volume-x.svg"),
            Self::Image => include_str!("../../assets/icons/lucide/image.svg"),
            Self::Check => include_str!("../../assets/icons/lucide/check.svg"),
            Self::Alert => include_str!("../../assets/icons/lucide/circle-alert.svg"),
        }
    }
}

pub fn button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let response = ui.add(
        egui::Button::new("")
            .min_size(vec2(34.0, 34.0))
            .selected(selected),
    );

    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label)
    });

    paint(
        ui.painter(),
        icon,
        response.rect.center(),
        18.0,
        if ui.is_enabled() {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color().gamma_multiply(0.5)
        },
    );

    response.on_hover_text(label)
}

pub fn logo(ui: &mut Ui, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    flower(ui.painter(), rect.center(), size * 0.43, color);
    response
}

pub fn flower(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    for n in 0..6 {
        let a = n as f32 * std::f32::consts::TAU / 6.0;
        painter.circle_filled(
            center + vec2(a.cos(), a.sin()) * radius * 0.56,
            radius * 0.44,
            color,
        );
    }
    painter.circle_filled(center, radius * 0.28, color);
}

pub fn paint(
    painter: &egui::Painter,
    icon: Icon,
    center: Pos2,
    size: f32,
    color: Color32,
) {
    icon_cache::paint(painter, icon, center, size, color);
}

pub fn swatch(ui: &mut Ui, color: Color32, selected: bool, label: &str) -> Response {
    let response = ui.add(
        egui::Button::new("")
            .fill(color)
            .min_size(vec2(31., 31.))
            .corner_radius(16.),
    );

    if selected {
        let check = if theme::is_dark(color) {
            Color32::WHITE
        } else {
            Color32::from_rgb(20, 22, 30)
        };
        paint(
            ui.painter(),
            Icon::Check,
            response.rect.center(),
            17.,
            check,
        );
    }

    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}
