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

const SLIDER_WIDTH: f32 = 180.;

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

    ui.spacing_mut().slider_width = SLIDER_WIDTH;
    ui.spacing_mut().item_spacing.y = space::XS;

    for control in &CONTROLS {
        control_row(ui, control, &mut edited, colors);
    }

    ui.add_space(space::SM);
    let reset = ui.add_enabled(!edited.is_neutral(), egui::Button::new("Reset all"));

    if reset.clicked() {
        edited = Adjustments::NEUTRAL;
    }

    if edited != current {
        commands.push(ViewCommand::Adjust(edited).into());
    }
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

    let reset_clicked = ui
        .horizontal(|ui| heading(ui, control.label, changed, colors))
        .inner;
    let slider_reset = slider(ui, control, value);

    if reset_clicked || slider_reset {
        *value = neutral;
    }
}

/// The label, and a reset button that appears once the value differs from neutral
fn heading(ui: &mut egui::Ui, label: &str, changed: bool, colors: Colors) -> bool {
    ui.label(RichText::new(label).size(text::LABEL).color(colors.text));

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let hint = format!("Reset {}", label.to_lowercase());

        // Hidden buttons keep their space, so the rows never shift while editing
        ui.scope(|ui| {
            if !changed {
                ui.set_invisible();
            }

            icons::small_button(ui, Icon::Reset, &hint).clicked()
        })
        .inner
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
            .suffix(control.suffix)
            .handle_shape(HandleShape::Circle),
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
