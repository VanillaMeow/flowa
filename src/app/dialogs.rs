//! File confirmations and temporary notifications

use super::*;

impl Flowa {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        let response = egui::Modal::new(egui::Id::unique("file-dialog")).show(ctx, |ui| {
            ui.set_width(400.);
            match &mut dialog {
                Dialog::Rename { path, name } => {
                    ui.heading("Rename image");
                    ui.add_space(12.);
                    ui.label("New filename");
                    let id = ui.scope_id().with("rename-filename");
                    // TextEdit consumes Enter, so capture it before drawing the field
                    let submit = ui.memory(|m| m.has_focus(id))
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let edit = ui.add(
                        egui::TextEdit::singleline(name)
                            .id(id)
                            .desired_width(f32::INFINITY),
                    );
                    if ui.memory(|m| m.focused().is_none()) {
                        edit.request_focus();
                    }
                    let valid = catalog::valid_filename(name);
                    if !valid {
                        ui.label(
                            RichText::new(
                                "Use a filename without path separators or reserved characters.",
                            )
                            .color(self.colors.muted),
                        );
                    }
                    ui.add_space(16.);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                        if ui.add_enabled(valid, egui::Button::new("Rename")).clicked()
                            || (valid && submit)
                        {
                            self.actions.request(Task::Rename {
                                source: path.clone(),
                                name: name.clone(),
                            });
                            self.busy = Some("Renaming image…".into());
                            keep = false;
                        }
                    });
                }
                Dialog::Trash(path) => {
                    ui.heading("Move this image to trash?");
                    ui.add_space(12.);
                    ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                    ui.add_space(8.);
                    ui.label(
                        RichText::new("You can restore it from your system’s trash.")
                            .color(self.colors.muted),
                    );
                    ui.add_space(18.);
                    ui.horizontal(|ui| {
                        if ui.button("Keep image").clicked() {
                            keep = false;
                        }
                        if ui.button("Move to trash").clicked() {
                            self.actions.request(Task::Trash(path.clone()));
                            self.busy = Some("Moving to trash…".into());
                            keep = false;
                        }
                    });
                }
            }
        });

        if response.should_close() {
            keep = false;
        }
        if keep {
            self.dialog = Some(dialog);
        }
    }

    pub(super) fn notice(&mut self, ctx: &egui::Context) {
        let Some(notice) = self.notice.as_ref() else {
            return;
        };
        if Instant::now() >= notice.until {
            self.notice = None;
            return;
        }
        let message = notice.message.clone();
        let error = notice.error;
        egui::Area::new(egui::Id::unique("notice"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0., -48.))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .stroke(Stroke::new(
                        1.,
                        if error {
                            Color32::from_rgb(230, 125, 125)
                        } else {
                            self.colors.border
                        },
                    ))
                    .corner_radius(10)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if error {
                                ui.label(
                                    RichText::new("!")
                                        .color(Color32::from_rgb(230, 125, 125)),
                                );
                            }
                            ui.add(egui::Label::new(message).wrap().selectable(true))
                                .on_hover_text("Notification");
                            if icons::button(
                                ui,
                                Icon::Close,
                                "Dismiss notification",
                                false,
                            )
                            .clicked()
                            {
                                self.notice = None;
                            }
                        });
                    });
            });
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}
