//! Viewer menus, toolbar controls, and the status bar

use super::*;

impl Flowa {
    pub(super) fn toolbar(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        egui::Panel::top("toolbar")
            .exact_size(58.)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .inner_margin(egui::Margin::symmetric(14, 9)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    icons::logo(ui, 26., self.colors.accent);
                    ui.add_space(4.);
                    ui.menu_button("File", |ui| self.file_menu(ui, ctx));
                    if icons::button(ui, Icon::Open, "Open image · Ctrl+O", false)
                        .clicked()
                    {
                        self.pick(false);
                    }
                    ui.separator();
                    ui.add_enabled_ui(self.files.len() > 1, |ui| {
                        if icons::button(
                            ui,
                            Icon::Left,
                            "Previous image · Left arrow",
                            false,
                        )
                        .clicked()
                        {
                            self.navigate(-1, ctx);
                        }
                        if icons::button(
                            ui,
                            Icon::Right,
                            "Next image · Right arrow",
                            false,
                        )
                        .clicked()
                        {
                            self.navigate(1, ctx);
                        }
                    });
                    let narrow = ui.available_width() < 760.;
                    if !narrow {
                        let title =
                            self.current().map(|e| e.name.as_str()).unwrap_or("flowa");
                        let width = (ui.available_width() - 480.0).clamp(40.0, 260.0);
                        ui.add_sized(
                            [width, 34.],
                            egui::Label::new(RichText::new(title).size(15.)).truncate(),
                        )
                        .on_hover_text(
                            self.current()
                                .map(|e| e.path.display().to_string())
                                .unwrap_or_else(|| {
                                    "An open space for your images".into()
                                }),
                        );
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            self.window_buttons(ui, ctx);
                            ui.separator();
                            ui.add_enabled_ui(self.decoded.is_some(), |ui| {
                                self.view_buttons(ui, ctx, narrow);
                            });
                        },
                    );
                });
            });
    }

    fn file_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("Open image…                    Ctrl+O").clicked() {
            ui.close();
            self.pick(false);
        }
        if ui.button("Open folder…             Ctrl+Shift+O").clicked() {
            ui.close();
            self.pick(true);
        }
        if ui
            .add_enabled(
                self.folder.is_some(),
                egui::Button::new("Refresh folder                              F5"),
            )
            .clicked()
        {
            if let Some(path) = self
                .current()
                .map(|e| e.path.clone())
                .or_else(|| self.folder.clone())
            {
                self.open_path(path, ctx);
            }
            ui.close();
        }
        ui.separator();
        ui.add_enabled_ui(self.decoded.is_some() && self.busy.is_none(), |ui| {
            if ui
                .button("Copy image                         Ctrl+C")
                .clicked()
            {
                ui.close();
                self.copy_image();
            }
            if ui
                .button("Export a copy…             Ctrl+Shift+S")
                .clicked()
            {
                ui.close();
                self.export();
            }
        });
        ui.add_enabled_ui(self.current().is_some() && self.busy.is_none(), |ui| {
            if ui
                .button("Rename…                                F2")
                .clicked()
            {
                ui.close();
                self.rename_dialog();
            }
            if ui
                .button("Move to trash…                     Delete")
                .clicked()
            {
                ui.close();
                self.trash_dialog();
            }
            if ui.button("Show folder in file manager").clicked()
                && let Some(entry) = self.current()
            {
                self.actions.request(Task::Reveal(entry.path.clone()));
                ui.close();
            }
        });
        ui.separator();
        if ui
            .button("Preferences…                     Ctrl+,")
            .clicked()
        {
            self.preferences_open = true;
            ui.close();
        }
        if ui
            .button("Quit                                  Ctrl+Q")
            .clicked()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn window_buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if icons::button(ui, Icon::Fullscreen, "Fullscreen · F11", self.fullscreen)
            .clicked()
        {
            self.toggle_fullscreen(ctx);
        }
        if icons::button(
            ui,
            Icon::Settings,
            "Preferences · Ctrl+,",
            self.preferences_open,
        )
        .clicked()
        {
            self.preferences_open = !self.preferences_open;
        }
        if icons::button(ui, Icon::Info, "Image details · I", self.settings.info)
            .clicked()
        {
            self.settings.info = !self.settings.info;
        }
        if icons::button(
            ui,
            Icon::Filmstrip,
            "Thumbnail strip · T",
            self.settings.filmstrip,
        )
        .clicked()
        {
            self.settings.filmstrip = !self.settings.filmstrip;
            self.scroll_to_current = true;
        }
    }

    fn view_buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, narrow: bool) {
        let animated = self.decoded.as_ref().is_some_and(|d| d.frames.len() > 1);
        if animated || self.slideshow {
            let playing = if animated {
                self.playing
            } else {
                self.slideshow
            };
            if icons::button(
                ui,
                if playing { Icon::Pause } else { Icon::Play },
                if playing {
                    "Pause · Space"
                } else {
                    "Play · Space"
                },
                false,
            )
            .clicked()
            {
                self.toggle_play();
            }
        }
        if !narrow
            && icons::button(ui, Icon::Rotate, "Rotate clockwise · R", false).clicked()
        {
            self.view.rotation = (self.view.rotation + 1) % 4;
        }
        if icons::button(ui, Icon::Fit, "Fit image · 0", self.view.fit).clicked() {
            self.view.reset();
        }
        if !narrow
            && icons::button(
                ui,
                Icon::Actual,
                "Actual pixels · 1",
                !self.view.fit
                    && (self.view.zoom * ctx.pixels_per_point() - 1.).abs() < 0.001,
            )
            .clicked()
        {
            self.actual_pixels(ctx);
        }
        if icons::button(ui, Icon::Plus, "Zoom in · +", false).clicked() {
            self.view.zoom_at(
                1.25,
                self.canvas_rect.center(),
                self.canvas_rect.center(),
            );
        }
        let percentage =
            format!("{:.0}%", self.view.zoom * ctx.pixels_per_point() * 100.);
        if ui
            .add_sized(
                [62., 32.],
                egui::Button::new(RichText::new(percentage).size(13.)).frame(false),
            )
            .on_hover_text("Click for actual pixels")
            .clicked()
        {
            self.actual_pixels(ctx);
        }
        if icons::button(ui, Icon::Minus, "Zoom out · -", false).clicked() {
            self.view.zoom_at(
                0.8,
                self.canvas_rect.center(),
                self.canvas_rect.center(),
            );
        }
    }

    pub(super) fn status(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(32.)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .inner_margin(egui::Margin::symmetric(18, 4)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(decoded) = &self.decoded {
                        ui.label(
                            RichText::new(format!(
                                "{}  ×  {}",
                                decoded.size[0], decoded.size[1]
                            ))
                            .size(12.)
                            .color(self.colors.muted),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(&decoded.format)
                                .size(12.)
                                .color(self.colors.muted),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(catalog::human_size(decoded.file_size))
                                .size(12.)
                                .color(self.colors.muted),
                        );
                        if decoded.frames.len() > 1 {
                            ui.separator();
                            ui.label(
                                RichText::new(format!(
                                    "{} frames",
                                    decoded.frames.len()
                                ))
                                .size(12.)
                                .color(self.colors.muted),
                            );
                        }
                    } else {
                        ui.label(
                            RichText::new("Drop an image or folder to begin")
                                .size(12.)
                                .color(self.colors.muted),
                        );
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if !self.files.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "{} / {}",
                                        self.index + 1,
                                        self.files.len()
                                    ))
                                    .size(12.)
                                    .color(self.colors.muted),
                                );
                            }
                            if let Some(busy) = &self.busy {
                                ui.spinner();
                                ui.label(
                                    RichText::new(busy)
                                        .size(12.)
                                        .color(self.colors.muted),
                                );
                            } else if self.slideshow {
                                ui.label(
                                    RichText::new("Slideshow")
                                        .size(12.)
                                        .color(self.colors.accent),
                                );
                            }
                        },
                    );
                });
            });
    }

    pub(super) fn context_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("Fit image                         0").clicked() {
            self.view.reset();
            ui.close();
        }
        if ui.button("Actual pixels                    1").clicked() {
            self.actual_pixels(ctx);
            ui.close();
        }
        if ui.button("Rotate clockwise              R").clicked() {
            self.view.rotation = (self.view.rotation + 1) % 4;
            ui.close();
        }
        if ui.button("Flip horizontally               H").clicked() {
            self.view.flip_x = !self.view.flip_x;
            ui.close();
        }
        if ui.button("Flip vertically                   V").clicked() {
            self.view.flip_y = !self.view.flip_y;
            ui.close();
        }
        ui.separator();
        if ui
            .button(if self.slideshow {
                "Stop slideshow"
            } else {
                "Start slideshow"
            })
            .clicked()
        {
            self.slideshow = !self.slideshow;
            self.next_slide = Instant::now()
                + Duration::from_secs_f32(self.settings.slideshow_seconds);
            ui.close();
        }
        if self.decoded.as_ref().is_some_and(|d| d.frames.len() > 1)
            && ui
                .button(if self.playing {
                    "Pause animation"
                } else {
                    "Play animation"
                })
                .clicked()
        {
            self.toggle_play();
            ui.close();
        }
        ui.separator();
        if ui.button("Copy image                  Ctrl+C").clicked() {
            self.copy_image();
            ui.close();
        }
        if ui.button("Export a copy…").clicked() {
            self.export();
            ui.close();
        }
        if ui.button("Rename…").clicked() {
            self.rename_dialog();
            ui.close();
        }
        if ui.button("Move to trash…").clicked() {
            self.trash_dialog();
            ui.close();
        }
        ui.separator();
        ui.menu_button("Sort folder", |ui| {
            let old = (self.settings.sort, self.settings.reverse_sort);
            for sort in [SortOrder::Name, SortOrder::Modified, SortOrder::Size] {
                ui.selectable_value(&mut self.settings.sort, sort, sort.name());
            }
            ui.separator();
            ui.checkbox(&mut self.settings.reverse_sort, "Reverse order");
            if old != (self.settings.sort, self.settings.reverse_sort) {
                self.sort();
            }
        });
    }
}
