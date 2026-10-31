//! File confirmations and temporary notifications

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use egui::{Color32, RichText, Stroke, vec2};

use crate::{app::Command, utils::filename};

use super::{
    icons::{self, Icon},
    theme::Colors,
};

struct Notice {
    message: String,
    error: bool,
    until: Instant,
}

enum Dialog {
    Rename { path: PathBuf, name: String },
    Trash(PathBuf),
}

#[derive(Default)]
pub(crate) struct Dialogs {
    active: Option<Dialog>,
    notice: Option<Notice>,
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    pub fn rename(&mut self, path: PathBuf, name: String) {
        self.active = Some(Dialog::Rename { path, name });
    }

    pub fn trash(&mut self, path: PathBuf) {
        self.active = Some(Dialog::Trash(path));
    }

    pub fn notify(&mut self, message: impl Into<String>, error: bool) {
        let duration = Duration::from_secs(if error { 12 } else { 4 });

        self.notice = Some(Notice {
            message: message.into(),
            error,
            until: Instant::now() + duration,
        });
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        let Some(mut dialog) = self.active.take() else {
            return;
        };

        let response =
            egui::Modal::new(egui::Id::unique("file-dialog")).show(ctx, |ui| {
                ui.set_width(400.);

                match &mut dialog {
                    Dialog::Rename { path, name } => {
                        rename_form(ui, path, name, colors, commands)
                    }
                    Dialog::Trash(path) => {
                        trash_confirmation(ui, path, colors, commands)
                    }
                }
            });

        let should_close = response.should_close();
        let keep_open = response.inner && !should_close;

        if keep_open {
            self.active = Some(dialog);
        }
    }

    pub fn show_notice(&mut self, ctx: &egui::Context, colors: Colors) {
        let Some(notice) = self.notice.as_ref() else {
            return;
        };

        if Instant::now() >= notice.until {
            self.notice = None;
            return;
        }

        let dismissed = egui::Area::new(egui::Id::unique("notice"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0., -48.))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| notice_frame(ui, notice, colors))
            .inner;

        if dismissed {
            self.notice = None;
        }

        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

fn rename_form(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    name: &mut String,
    colors: Colors,
    commands: &mut Vec<Command>,
) -> bool {
    ui.heading("Rename image");
    ui.add_space(12.);
    ui.label("New filename");

    let id = ui.scope_id().with("rename-filename");

    // TextEdit consumes Enter, so capture it before drawing the field
    let submit = ui.memory(|memory| memory.has_focus(id))
        && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let edit = ui.add(
        egui::TextEdit::singleline(name)
            .id(id)
            .desired_width(f32::INFINITY),
    );

    if ui.memory(|memory| memory.focused().is_none()) {
        edit.request_focus();
    }

    let valid = filename::is_valid(name);

    if !valid {
        let hint = "Use a filename without path separators or reserved characters.";
        ui.label(RichText::new(hint).color(colors.muted));
    }

    ui.add_space(16.);
    ui.horizontal(|ui| {
        let mut keep_open = true;

        if ui.button("Cancel").clicked() {
            keep_open = false;
        }

        let rename = ui.add_enabled(valid, egui::Button::new("Rename")).clicked();

        if rename || (valid && submit) {
            commands.push(Command::ConfirmRename {
                path: path.to_path_buf(),
                name: name.clone(),
            });
            keep_open = false;
        }

        keep_open
    })
    .inner
}

fn trash_confirmation(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    colors: Colors,
    commands: &mut Vec<Command>,
) -> bool {
    ui.heading("Move this image to trash?");
    ui.add_space(12.);
    ui.label(path.file_name().unwrap_or_default().to_string_lossy());
    ui.add_space(8.);
    ui.label(
        RichText::new("You can restore it from your system’s trash.")
            .color(colors.muted),
    );

    ui.add_space(18.);
    ui.horizontal(|ui| {
        let mut keep_open = true;

        if ui.button("Keep image").clicked() {
            keep_open = false;
        }

        if ui.button("Move to trash").clicked() {
            commands.push(Command::ConfirmTrash(path.to_path_buf()));
            keep_open = false;
        }

        keep_open
    })
    .inner
}

fn notice_frame(ui: &mut egui::Ui, notice: &Notice, colors: Colors) -> bool {
    let border = if notice.error {
        error_color()
    } else {
        colors.border
    };

    egui::Frame::new()
        .fill(colors.panel)
        .stroke(Stroke::new(1., border))
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| notice_row(ui, notice))
        .inner
}

fn notice_row(ui: &mut egui::Ui, notice: &Notice) -> bool {
    ui.horizontal(|ui| {
        if notice.error {
            let (rect, _) =
                ui.allocate_exact_size(vec2(16., 18.), egui::Sense::hover());
            icons::paint(ui.painter(), Icon::Alert, rect.center(), 16., error_color());
        }

        ui.add(egui::Label::new(&notice.message).wrap().selectable(true))
            .on_hover_text("Notification");

        icons::button(ui, Icon::Close, "Dismiss notification", false).clicked()
    })
    .inner
}

fn error_color() -> Color32 {
    Color32::from_rgb(230, 125, 125)
}

#[cfg(test)]
mod tests {
    use crate::{
        settings::{Accent, Theme},
        ui::theme,
    };

    use super::*;

    #[test]
    fn rename_enter_returns_the_path_and_name_from_the_dialog() {
        let ctx = egui::Context::default();
        let colors = theme::colors(Theme::Nord, Accent::Theme, true);
        let mut dialogs = Dialogs::default();
        let mut commands = Vec::new();
        let path = PathBuf::from("original.png");
        dialogs.rename(path.clone(), "renamed.png".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
            dialogs.show(&ctx, colors, &mut commands)
        });
        output.textures_delta.clear();
        assert!(commands.is_empty());

        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output =
            ctx.run_ui(input, |_| dialogs.show(&ctx, colors, &mut commands));
        output.textures_delta.clear();
        assert_eq!(
            commands,
            vec![Command::ConfirmRename {
                path,
                name: "renamed.png".into()
            }]
        );
        assert!(!dialogs.is_open());
    }
}
