//! File properties and photographic metadata

use crate::{
    app::{Command, WindowCommand},
    utils::format,
    video::VideoState,
    viewer::{
        catalog::Entry,
        formats::{self, MediaKind},
        image_session::DecodedImage,
    },
};

use super::{
    actions,
    icons::{self, Icon},
    style::{margin, space},
    theme::Colors,
    widgets,
};

pub fn show(
    root: &mut egui::Ui,
    entry: Option<&Entry>,
    decoded: Option<&DecodedImage>,
    video: &VideoState,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    egui::Panel::right("inspector")
        .default_size(280.)
        .size_range(230.0..=420.0)
        .frame(
            egui::Frame::new()
                .fill(colors.panel)
                .inner_margin(margin::SIDE_PANEL),
        )
        .show(root, |ui| {
            header(ui, commands);
            ui.add_space(space::LG);

            egui::ScrollArea::vertical().show(ui, |ui| {
                details(ui, entry, decoded, video, colors, commands);
            });
        });
}

fn header(ui: &mut egui::Ui, commands: &mut Vec<Command>) {
    ui.horizontal(|ui| {
        ui.heading("File details");

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, Icon::Close, "Close file details", false).clicked() {
                commands.push(WindowCommand::ToggleInspector.into());
            }
        });
    });
}

fn details(
    ui: &mut egui::Ui,
    entry: Option<&Entry>,
    decoded: Option<&DecodedImage>,
    video: &VideoState,
    colors: Colors,
    commands: &mut Vec<Command>,
) {
    if let Some(entry) = entry {
        file_details(ui, entry, colors);
    }

    if let Some(decoded) = decoded {
        image_details(ui, decoded, colors);
    }

    if video.active {
        video_details(ui, video, colors);
    }

    ui.add_space(space::SM);

    if entry.is_some() {
        file_actions(ui, commands);
    }
}

fn file_details(ui: &mut egui::Ui, entry: &Entry, colors: Colors) {
    let folder = entry
        .path
        .parent()
        .map(|path| path.display().to_string())
        .unwrap_or_default();

    detail(ui, "FILE", &entry.name, colors);
    detail(ui, "FOLDER", &folder, colors);

    if entry.kind == MediaKind::Video {
        let container = formats::label(&entry.path).unwrap_or_default();
        detail(ui, "CONTAINER", &container, colors);
        detail(ui, "FILE SIZE", &format::file_size(entry.size), colors);
    }
}

fn video_details(ui: &mut egui::Ui, video: &VideoState, colors: Colors) {
    if !video.size.contains(&0) {
        let dimensions = format!("{} × {} pixels", video.size[0], video.size[1]);
        detail(ui, "DIMENSIONS", &dimensions, colors);
    }

    if video.duration > 0. {
        detail(ui, "DURATION", &format::media_time(video.duration), colors);
    }

    if !video.codec.is_empty() {
        detail(ui, "VIDEO CODEC", video.codec_description(), colors);
        audio_details(ui, video, colors);
    }

    detail(ui, "DECODER", &video.decoder_description(), colors);
    detail(ui, "RENDERER", "Vulkan · libplacebo / wgpu", colors);

    if let Some(error) = &video.error {
        detail(ui, "PLAYBACK ERROR", error, colors);
    }
}

fn audio_details(ui: &mut egui::Ui, video: &VideoState, colors: Colors) {
    let Some(audio) = &video.audio else {
        detail(ui, "AUDIO", "No audio track", colors);
        return;
    };
    detail(ui, "AUDIO CODEC", &audio.codec.to_uppercase(), colors);
    let channels = match audio.channels {
        1 => "Mono".into(),
        2 => "Stereo".into(),
        count => format!("{count} channels"),
    };
    detail(ui, "AUDIO CHANNELS", &channels, colors);
    if audio.sample_rate > 0 {
        detail(
            ui,
            "SAMPLE RATE",
            &format!("{} Hz", audio.sample_rate),
            colors,
        );
    }
    if audio.bit_rate > 0 {
        detail(
            ui,
            "AUDIO BITRATE",
            &format!("{} kb/s", audio.bit_rate / 1000),
            colors,
        );
    }
}

fn image_details(ui: &mut egui::Ui, image: &DecodedImage, colors: Colors) {
    let dimensions = format!("{} × {} pixels", image.size[0], image.size[1]);
    let file_size = format::file_size(image.file_size);

    detail(ui, "DIMENSIONS", &dimensions, colors);
    detail(ui, "FORMAT", &image.format, colors);
    detail(ui, "FILE SIZE", &file_size, colors);
    detail(ui, "COLOR", &image.color, colors);

    if image.frames.len() > 1 {
        let animation = format!(
            "{} frames · {:.2} seconds",
            image.frames.len(),
            image.duration().as_secs_f64(),
        );

        detail(ui, "ANIMATION", &animation, colors);
    }

    for (key, value) in &image.metadata {
        detail(ui, key, value, colors);
    }
}

fn file_actions(ui: &mut egui::Ui, commands: &mut Vec<Command>) {
    for action in [&actions::COPY_PATH, &actions::REVEAL] {
        if ui.button(action.label).clicked() {
            commands.push(action.command.clone());
        }
    }
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str, colors: Colors) {
    widgets::caption(ui, label, colors);
    ui.add(egui::Label::new(value).wrap());
    ui.add_space(space::MD);
}
