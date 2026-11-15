//! Brightness, contrast, saturation, and gamma sliders for the viewed image or video

use std::ops::RangeInclusive;

use egui::{RichText, Vec2, style::HandleShape};

use crate::{
    app::{Command, ViewCommand},
    viewer::adjustments::{self, Adjustments},
};

use super::{
    icons::{self, Icon},
    style::{size, space, text},
    theme::Colors,
};

const SLIDER_WIDTH: f32 = 200.;

/// One slider's label, range, and how its value reads to the user
struct Control {
    label: &'static str,
    range: RangeInclusive<f32>,
    logarithmic: bool,
    format: fn(f32) -> String,
}

const BRIGHTNESS: Control = Control {
    label: "Brightness",
    range: adjustments::BRIGHTNESS,
    logarithmic: false,
    format: |value| format!("{:+.0}", value * 200.),
};
const CONTRAST: Control = Control {
    label: "Contrast",
    range: adjustments::CONTRAST,
    logarithmic: false,
    format: |value| format!("{:.0}%", value * 100.),
};
const SATURATION: Control = Control {
    label: "Saturation",
    range: adjustments::SATURATION,
    logarithmic: false,
    format: |value| format!("{:.0}%", value * 100.),
};
const GAMMA: Control = Control {
    label: "Gamma",
    range: adjustments::GAMMA,
    logarithmic: true,
    format: |value| format!("{value:.2}"),
};

/// A toolbar button that opens the sliders, highlighted while any value is changed
pub fn button(
    ui: &mut egui::Ui,
    current: Adjustments,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    let button = egui::Button::new("")
        .min_size(Vec2::splat(size::CONTROL))
        .selected(!current.is_neutral());
    let (response, _) = egui::containers::menu::MenuButton::from_button(button)
        .ui(ui, |ui| controls(ui, current, colors, commands));

    icons::paint(
        ui.painter(),
        Icon::Adjust,
        response.rect.center(),
        size::ICON,
        icon_color(ui),
    );

    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Adjust colors")
    });
    response.on_hover_text("Adjust colors");
}

/// The sliders and a reset button, for the toolbar popup and context menus
pub fn controls(
    ui: &mut egui::Ui,
    current: Adjustments,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    let mut edited = current;

    ui.spacing_mut().slider_width = SLIDER_WIDTH;
    ui.spacing_mut().item_spacing.y = space::XS;

    slider(ui, &BRIGHTNESS, &mut edited.brightness, colors);
    slider(ui, &CONTRAST, &mut edited.contrast, colors);
    slider(ui, &SATURATION, &mut edited.saturation, colors);
    slider(ui, &GAMMA, &mut edited.gamma, colors);

    ui.add_space(space::SM);
    let reset = ui.add_enabled(!edited.is_neutral(), egui::Button::new("Reset"));

    if reset.clicked() {
        edited = Adjustments::NEUTRAL;
    }

    if edited != current {
        commands.push(ViewCommand::Adjust(edited).into());
    }
}

fn slider(ui: &mut egui::Ui, control: &Control, value: &mut f32, colors: Colors) {
    ui.horizontal(|ui| {
        ui.set_width(SLIDER_WIDTH);
        slider_heading(ui, control, *value, colors);
    });

    let response = ui.add(
        egui::Slider::new(value, control.range.clone())
            .logarithmic(control.logarithmic)
            .show_value(false)
            .handle_shape(HandleShape::Circle),
    );
    response.widget_info(|| {
        egui::WidgetInfo::slider(true, f64::from(*value), control.label)
    });
}

fn slider_heading(ui: &mut egui::Ui, control: &Control, value: f32, colors: Colors) {
    ui.label(RichText::new(control.label).size(text::LABEL));

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let reading = (control.format)(value);
        ui.label(RichText::new(reading).size(text::LABEL).color(colors.muted));
    });
}

fn icon_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.is_enabled() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    }
}
