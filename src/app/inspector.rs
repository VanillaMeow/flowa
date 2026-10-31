//! File properties and photographic metadata

use super::*;

impl Flowa {
    pub(super) fn inspector(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        egui::Panel::right("inspector")
            .default_size(280.)
            .size_range(230.0..=420.0)
            .frame(egui::Frame::new().fill(self.colors.panel).inner_margin(20))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Image details");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Close, "Close image details", false).clicked() {
                            self.settings.info = false;
                        }
                    });
                });
                ui.add_space(18.);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(entry) = self.current() {
                        detail(ui, "FILE", &entry.name, self.colors);
                        detail(
                            ui,
                            "FOLDER",
                            &entry
                                .path
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default(),
                            self.colors,
                        );
                    }
                    if let Some(decoded) = &self.decoded {
                        detail(
                            ui,
                            "DIMENSIONS",
                            &format!("{} × {} pixels", decoded.size[0], decoded.size[1]),
                            self.colors,
                        );
                        detail(ui, "FORMAT", &decoded.format, self.colors);
                        detail(
                            ui,
                            "FILE SIZE",
                            &catalog::human_size(decoded.file_size),
                            self.colors,
                        );
                        detail(ui, "COLOR", &decoded.color, self.colors);
                        if decoded.frames.len() > 1 {
                            detail(
                                ui,
                                "ANIMATION",
                                &format!(
                                    "{} frames · {:.2} seconds",
                                    decoded.frames.len(),
                                    decoded.duration().as_secs_f64()
                                ),
                                self.colors,
                            );
                        }
                        for (key, value) in &decoded.metadata {
                            detail(ui, key, value, self.colors);
                        }
                    }
                    ui.add_space(10.);
                    if let Some(path) = self.current().map(|e| e.path.clone()) {
                        if ui.button("Copy file path").clicked() {
                            ctx.copy_text(path.to_string_lossy().into_owned());
                            self.notify("Path copied", false);
                        }
                        if ui.button("Open containing folder").clicked() {
                            self.actions.request(Task::Reveal(path));
                        }
                    }
                    ui.add_space(20.);
                    ui.label(
                        RichText::new(
                            "Rotation and flips are temporary. Export a copy to save what you see.",
                        )
                        .size(13.)
                        .color(self.colors.muted),
                    );
                });
            });
    }
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str, c: Colors) {
    ui.label(RichText::new(label).size(11.).color(c.muted));
    ui.add(egui::Label::new(value).wrap());
    ui.add_space(15.);
}
