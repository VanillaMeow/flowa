//! Choosing the audio track of the playing video

use egui::{RichText, Vec2};

use crate::{
    app::{Command, PlaybackCommand},
    video::{Track, VideoState},
};

use super::{
    icons::{self, Icon},
    style::{size, text},
    theme::Colors,
};

/// Whether the video offers anything to choose between
pub fn available(state: &VideoState) -> bool {
    state.tracks.audio.len() > 1
}

/// An icon button on the video controls that opens the track lists
pub fn button(
    ui: &mut egui::Ui,
    state: &VideoState,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    let button = egui::Button::new("").min_size(Vec2::splat(size::CONTROL));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button)
        .ui(ui, |ui| menu(ui, state, colors, commands));

    let color = if ui.is_enabled() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    icons::paint(
        ui.painter(),
        Icon::Tracks,
        response.rect.center(),
        size::ICON,
        color,
    );

    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Tracks")
    });
    response.on_hover_text("Tracks");
}

/// Every track list, each under a caption
pub fn menu(
    ui: &mut egui::Ui,
    state: &VideoState,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    if state.tracks.audio.len() > 1 {
        caption(ui, "Audio", colors);
        audio_options(ui, state, commands);
    }
}

/// The audio tracks as selectable rows
pub fn audio_options(
    ui: &mut egui::Ui,
    state: &VideoState,
    commands: &mut Vec<Command>,
) {
    for track in &state.tracks.audio {
        let selected = state.audio_track == Some(track.index);

        if option(ui, track, selected).clicked() && !selected {
            commands.push(PlaybackCommand::SelectAudio(track.index).into());
            ui.close();
        }
    }
}

fn option(ui: &mut egui::Ui, track: &Track, selected: bool) -> egui::Response {
    ui.selectable_label(selected, &track.label)
}

fn caption(ui: &mut egui::Ui, label: &str, colors: Colors) {
    ui.label(
        RichText::new(label.to_uppercase())
            .size(text::CAPTION)
            .color(colors.muted),
    );
}
