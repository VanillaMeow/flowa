//! Reusable building blocks that give every panel the same look

use egui::{RichText, Stroke, Ui};

use super::{
    style::{margin, motion, radius, space, text},
    theme::{self, Colors},
};

/// A captioned card that groups related settings or details
pub fn section<R>(
    ui: &mut Ui,
    title: &str,
    colors: Colors,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    caption(ui, title, colors);
    ui.add_space(space::XS);

    // Recessed toward the canvas so raised controls inside keep their contrast
    let inner = egui::Frame::new()
        .fill(theme::mix(colors.panel, colors.canvas, 0.5))
        .stroke(Stroke::new(1., colors.border.gamma_multiply(0.6)))
        .corner_radius(radius::CARD)
        .inner_margin(margin::SECTION)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner;

    ui.add_space(space::LG);
    inner
}

/// A small uppercase label above a value or section
pub fn caption(ui: &mut Ui, label: &str, colors: Colors) {
    ui.label(
        RichText::new(label.to_uppercase())
            .size(text::CAPTION)
            .color(colors.muted),
    );
}

/// Secondary text explaining the control above it
pub fn hint(ui: &mut Ui, label: &str, colors: Colors) {
    ui.label(RichText::new(label).size(text::LABEL).color(colors.muted));
}

/// A row of mutually exclusive options on one shared backdrop
///
/// Returns true when the selection changed
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    selected: &mut T,
    options: impl IntoIterator<Item = (T, &'static str)>,
    colors: Colors,
) -> bool {
    let mut changed = false;

    egui::Frame::new()
        .fill(colors.raised)
        .corner_radius(radius::CONTROL + 2)
        .inner_margin(3)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.;

            ui.horizontal_wrapped(|ui| {
                for (value, label) in options {
                    changed |= segment(ui, selected, value, label);
                }
            });
        });

    changed
}

fn segment<T: Copy + PartialEq>(
    ui: &mut Ui,
    selected: &mut T,
    value: T,
    label: &str,
) -> bool {
    let active = *selected == value;
    let mut button = egui::Button::new(label)
        .stroke(Stroke::NONE)
        .corner_radius(radius::CONTROL)
        .min_size(egui::vec2(0., 30.));

    // The selected option is lifted off the track; the others only show on hover
    button = if active {
        button.fill(ui.visuals().selection.bg_fill)
    } else {
        button.frame_when_inactive(false)
    };

    let clicked = ui.add(button).clicked() && !active;

    if clicked {
        *selected = value;
    }

    clicked
}

/// Opacity of an element that egui fades in, fading out briefly once it is hidden
pub fn fade_out(ctx: &egui::Context, id: egui::Id, visible: bool) -> f32 {
    let (target, duration) = if visible {
        (1., 0.)
    } else {
        (0., motion::FADE_OUT)
    };

    ctx.animate_value_with_time(id, target, duration)
}
