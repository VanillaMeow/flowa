//! Brightness, contrast, saturation, and gamma sliders for the viewed image or video

use std::ops::RangeInclusive;

use egui::{RichText, Vec2, vec2};

use crate::{
    app::{Command, ViewCommand},
    viewer::adjustments::{self, Adjustments},
};

use super::{
    icons::{self, Icon},
    style::{size, space, text},
    theme::Colors,
};

const SLIDER_WIDTH: f32 = 150.;
const ROW_HEIGHT: f32 = 20.;
/// Every number field gets the width of the widest reading, such as `200%`
const VALUE_WIDTH: f32 = 60.;

/// One slider: its label, the field it edits, and the units it shows
struct Control {
    label: &'static str,
    field: fn(&mut Adjustments) -> &mut f32,
    /// Range in displayed units
    range: RangeInclusive<f32>,
    /// Displayed units per stored unit
    scale: f32,
    step: f64,
    suffix: &'static str,
    logarithmic: bool,
}

const CONTROLS: [Control; 4] = [
    Control {
        label: "Brightness",
        field: |adjustments| &mut adjustments.brightness,
        range: -100.0..=100.0,
        scale: 200.,
        step: 1.,
        suffix: "",
        logarithmic: false,
    },
    Control {
        label: "Contrast",
        field: |adjustments| &mut adjustments.contrast,
        range: 0.0..=200.0,
        scale: 100.,
        step: 1.,
        suffix: "%",
        logarithmic: false,
    },
    Control {
        label: "Saturation",
        field: |adjustments| &mut adjustments.saturation,
        range: 0.0..=200.0,
        scale: 100.,
        step: 1.,
        suffix: "%",
        logarithmic: false,
    },
    Control {
        label: "Gamma",
        field: |adjustments| &mut adjustments.gamma,
        range: *adjustments::GAMMA.start()..=*adjustments::GAMMA.end(),
        scale: 1.,
        step: 0.01,
        suffix: "",
        logarithmic: true,
    },
];

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
///
/// Each slider resets with its reset button, a double-click, or Delete or Backspace
/// while focused
pub fn controls(
    ui: &mut egui::Ui,
    current: Adjustments,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    let mut edited = current;

    ui.scope(|ui| rows(ui, &mut edited, colors));

    ui.add_space(space::XS);
    let reset = ui.add_enabled(!edited.is_neutral(), egui::Button::new("Reset all"));

    if reset.clicked() {
        edited = Adjustments::NEUTRAL;
    }

    if edited != current {
        commands.push(ViewCommand::Adjust(edited).into());
    }
}

/// One row per value: label, slider and number field, then its reset button
fn rows(ui: &mut egui::Ui, edited: &mut Adjustments, colors: Colors) {
    let spacing = ui.spacing_mut();
    spacing.slider_width = SLIDER_WIDTH;
    spacing.slider_rail_height = 3.;
    spacing.interact_size = vec2(VALUE_WIDTH, ROW_HEIGHT);
    spacing.button_padding = vec2(space::XS, 1.);

    egui::Grid::new("color-adjustments")
        .num_columns(3)
        .spacing(vec2(space::SM, space::XS))
        .show(ui, |ui| {
            for control in &CONTROLS {
                control_row(ui, control, edited, colors);
                ui.end_row();
            }
        });
}

fn control_row(
    ui: &mut egui::Ui,
    control: &Control,
    edited: &mut Adjustments,
    colors: Colors,
) {
    let mut defaults = Adjustments::NEUTRAL;
    let neutral = *(control.field)(&mut defaults);
    let value = (control.field)(edited);
    let changed = (*value - neutral).abs() > f32::EPSILON;

    ui.label(
        RichText::new(control.label)
            .size(text::LABEL)
            .color(colors.text),
    );
    let slider_reset = slider(ui, control, value);
    let reset_clicked = reset_button(ui, control.label, changed);

    if reset_clicked || slider_reset {
        *value = neutral;
    }
}

/// A reset button that appears once the value differs from neutral
fn reset_button(ui: &mut egui::Ui, label: &str, changed: bool) -> bool {
    let hint = format!("Reset {}", label.to_lowercase());

    // A hidden button keeps its space, so the rows never shift while editing
    ui.scope(|ui| {
        if !changed {
            ui.set_invisible();
        }

        icons::small_button(ui, Icon::Reset, &hint).clicked()
    })
    .inner
}

/// The slider with its number field; returns whether the user asked to reset it
fn slider(ui: &mut egui::Ui, control: &Control, value: &mut f32) -> bool {
    let mut shown = *value * control.scale;
    let response = ui.add(
        egui::Slider::new(&mut shown, control.range.clone())
            .logarithmic(control.logarithmic)
            .step_by(control.step)
            .suffix(control.suffix),
    );

    if response.changed() {
        *value = shown / control.scale;
    }

    let keyboard_reset = response.has_focus()
        && ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
        });

    let response = response.on_hover_text("Double-click or press Delete to reset");

    response.double_clicked() || keyboard_reset
}

fn icon_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.is_enabled() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    }
}
