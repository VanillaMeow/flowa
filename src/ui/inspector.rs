//! File properties and photographic metadata

use egui::RichText;

use crate::{
    app::Command,
    utils::format,
    video::VideoState,
    viewer::{catalog::Entry, image_session::DecodedImage},
};

use super::{
    icons::{self, Icon},
    theme::Colors,
};

pub(crate) fn show(
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
        .frame(egui::Frame::new().fill(colors.panel).inner_margin(20))
        .show(root, |ui| {
            header(ui, commands);
            ui.add_space(18.);

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
                commands.push(Command::ToggleInspector);
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

    ui.add_space(10.);

    if let Some(entry) = entry {
        file_actions(ui, entry, commands);
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

    if crate::viewer::catalog::is_video(&entry.path) {
        let container = entry
            .path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_uppercase();
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

    detail(ui, "DECODER", &video.decoder_description(), colors);
    detail(ui, "RENDERER", "Vulkan · wgpu / libmpv", colors);

    if let Some(error) = &video.error {
        detail(ui, "PLAYBACK ERROR", error, colors);
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

fn file_actions(ui: &mut egui::Ui, entry: &Entry, commands: &mut Vec<Command>) {
    if ui.button("Copy file path").clicked() {
        commands.push(Command::CopyPath);
    }

    if ui.button("Open containing folder").clicked() {
        commands.push(Command::Reveal(entry.path.clone()));
    }
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str, colors: Colors) {
    ui.label(RichText::new(label).size(11.).color(colors.muted));
    ui.add(egui::Label::new(value).wrap());
    ui.add_space(15.);
}
