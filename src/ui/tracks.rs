//! Choosing the audio and subtitle tracks of the playing video

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
    state.tracks.audio.len() > 1 || !state.tracks.subtitles.is_empty()
}

/// An icon button on the video controls that opens the track lists
pub fn button(
    ui: &mut egui::Ui,
    state: &VideoState,
    colors: Colors,
    commands: &mut Vec<Command>,
) -> egui::Response {
    let button = egui::Button::new("").min_size(Vec2::splat(size::CONTROL));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button)
        .ui(ui, |ui| menu(ui, state, colors, commands));

    let color = if ui.is_enabled() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let icon = if state.tracks.subtitles.is_empty() {
        Icon::Tracks
    } else {
        Icon::Captions
    };
    icons::paint(
        ui.painter(),
        icon,
        response.rect.center(),
        size::ICON,
        color,
    );

    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Tracks")
    });
    response.on_hover_text("Tracks")
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

    if !state.tracks.subtitles.is_empty() {
        if state.tracks.audio.len() > 1 {
            ui.separator();
        }

        caption(ui, "Subtitles", colors);
        subtitle_options(ui, state, commands);
    }
}

/// Off, then every subtitle track, as selectable rows
pub fn subtitle_options(
    ui: &mut egui::Ui,
    state: &VideoState,
    commands: &mut Vec<Command>,
) {
    let off = state.subtitle_track.is_none();

    if ui.selectable_label(off, "Off").clicked() && !off {
        commands.push(PlaybackCommand::SelectSubtitles(None).into());
        ui.close();
    }

    for track in &state.tracks.subtitles {
        let selected = state.subtitle_track == Some(track.index);

        if option(ui, track, selected).clicked() && !selected {
            commands.push(PlaybackCommand::SelectSubtitles(Some(track.index)).into());
            ui.close();
        }
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
