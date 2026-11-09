//! File confirmations and temporary notifications

use std::{path::PathBuf, time::Instant};

use egui::{Color32, Id, RichText, Stroke, vec2};

use crate::{
    app::{Command, FileCommand},
    utils::filename,
};

use super::{
    icons::{self, Icon},
    style::{margin, motion, radius, size, space, text},
    theme::Colors,
    widgets,
};

/// Long messages wrap instead of stretching the notice across the window
const NOTICE_MAX_WIDTH: f32 = 420.;

/// Keeps a hovered notice up so it can be read or selected
const NOTICE_HOVER_GRACE: std::time::Duration = std::time::Duration::from_millis(800);

struct Notice {
    message: String,
    error: bool,
    until: Instant,
    dismissed: bool,
}

#[derive(Clone)]
enum Dialog {
    Rename { path: PathBuf, name: String },
    Trash(PathBuf),
}

#[derive(Default)]
pub struct Dialogs {
    active: Option<Dialog>,
    /// A dialog that was just closed, drawn while it fades out
    closing: Option<Dialog>,
    notice: Option<Notice>,
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    pub fn rename(&mut self, path: PathBuf, name: String) {
        self.open(Dialog::Rename { path, name });
    }

    pub fn trash(&mut self, path: PathBuf) {
        self.open(Dialog::Trash(path));
    }

    fn open(&mut self, dialog: Dialog) {
        self.active = Some(dialog);
        self.closing = None;
    }

    pub fn notify(&mut self, message: impl Into<String>, error: bool) {
        let duration = if error {
            motion::ERROR_NOTICE
        } else {
            motion::NOTICE
        };

        self.notice = Some(Notice {
            message: message.into(),
            error,
            until: Instant::now() + duration,
            dismissed: false,
        });
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        // Fading starts the frame after the modal closes, so the two never overlap
        let opacity = widgets::fade_out(ctx, dialog_id().with("fade"), self.is_open());

        if self.active.is_none() {
            self.show_closing(ctx, colors, opacity);
        }

        let Some(mut dialog) = self.active.take() else {
            return;
        };

        let response = egui::Modal::new(dialog_id())
            .show(ctx, |ui| dialog_contents(ui, &mut dialog, colors, commands));

        if response.inner && !response.should_close() {
            self.active = Some(dialog);
        } else {
            self.closing = Some(dialog);
        }
    }

    /// Redraws the closed dialog without interaction while it fades away
    fn show_closing(&mut self, ctx: &egui::Context, colors: Colors, opacity: f32) {
        if opacity <= 0. {
            self.closing = None;
        }

        let Some(dialog) = &mut self.closing else {
            return;
        };

        egui::Modal::default_area(dialog_id().with("closing"))
            .interactable(false)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(opacity);

                let backdrop = ui.ctx().content_rect();
                ui.painter()
                    .rect_filled(backdrop, 0., Color32::from_black_alpha(100));

                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    dialog_contents(ui, dialog, colors, &mut Vec::new())
                });
            });
    }

    pub fn show_notice(&mut self, ctx: &egui::Context, colors: Colors) {
        let Some(notice) = &mut self.notice else {
            return;
        };

        let now = Instant::now();
        let visible = !notice.dismissed && now < notice.until;
        let opacity = widgets::fade_out(ctx, Id::unique("notice-fade"), visible);

        if opacity <= 0. {
            self.notice = None;
            return;
        }

        let area = egui::Area::new(Id::unique("notice"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0., -48.))
            .order(egui::Order::Foreground)
            .interactable(visible)
            .show(ctx, |ui| {
                ui.multiply_opacity(opacity);
                notice_frame(ui, notice, colors)
            });

        if area.inner {
            notice.dismissed = true;
        } else if visible && area.response.contains_pointer() {
            notice.until = notice.until.max(now + NOTICE_HOVER_GRACE);
        }

        if visible {
            ctx.request_repaint_after(notice.until.saturating_duration_since(now));
        }
    }
}

fn dialog_id() -> Id {
    Id::unique("file-dialog")
}

/// Returns false once the dialog has been answered
fn dialog_contents(
    ui: &mut egui::Ui,
    dialog: &mut Dialog,
    colors: Colors,
    commands: &mut Vec<Command>,
) -> bool {
    ui.set_width(400.);

    match dialog {
        Dialog::Rename { path, name } => rename_form(ui, path, name, colors, commands),
        Dialog::Trash(path) => trash_confirmation(ui, path, commands),
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
    ui.add_space(space::MD);
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

    ui.add_space(space::LG);
    ui.horizontal(|ui| {
        let mut keep_open = true;

        if ui.button("Cancel").clicked() {
            keep_open = false;
        }

        let rename = ui.add_enabled(valid, egui::Button::new("Rename")).clicked();

        if rename || (valid && submit) {
            let confirm = FileCommand::ConfirmRename {
                path: path.to_path_buf(),
                name: name.clone(),
            };
            commands.push(confirm.into());
            keep_open = false;
        }

        keep_open
    })
    .inner
}

fn trash_confirmation(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    commands: &mut Vec<Command>,
) -> bool {
    ui.heading("Move this image to trash?");
    ui.add_space(space::MD);
    ui.label(path.file_name().unwrap_or_default().to_string_lossy());

    ui.add_space(space::LG);
    ui.horizontal(|ui| {
        let mut keep_open = true;

        if ui.button("Keep image").clicked() {
            keep_open = false;
        }

        if ui.button("Move to trash").clicked() {
            commands.push(FileCommand::ConfirmTrash(path.to_path_buf()).into());
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
        .corner_radius(radius::CARD)
        .inner_margin(margin::NOTICE)
        .shadow(ui.visuals().popup_shadow)
        .show(ui, |ui| notice_row(ui, notice))
        .inner
}

fn notice_row(ui: &mut egui::Ui, notice: &Notice) -> bool {
    ui.set_max_width(NOTICE_MAX_WIDTH);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;

        if notice.error {
            let (rect, _) = ui.allocate_exact_size(
                vec2(size::SMALL_ICON, size::SMALL_ICON),
                egui::Sense::hover(),
            );
            let center = rect.center();
            icons::paint(
                ui.painter(),
                Icon::Alert,
                center,
                size::SMALL_ICON,
                error_color(),
            );
        }

        let message = RichText::new(&notice.message).size(text::LABEL);
        ui.add(egui::Label::new(message).wrap().selectable(true));

        icons::small_button(ui, Icon::Close, "Dismiss notification").clicked()
    })
    .inner
}

fn error_color() -> Color32 {
    Color32::from_rgb(230, 125, 125)
}
