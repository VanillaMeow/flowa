//! Image painting, transforms, and visible GPU tiles

use super::*;

impl Flowa {
    fn canvas_color(&self) -> Color32 {
        match self.settings.canvas {
            Canvas::Theme => self.colors.canvas,
            Canvas::Dark => Color32::from_rgb(24, 24, 26),
            Canvas::Black => Color32::BLACK,
            Canvas::White => Color32::WHITE,
            Canvas::Custom => Color32::from_rgb(
                self.settings.canvas_color[0],
                self.settings.canvas_color[1],
                self.settings.canvas_color[2],
            ),
        }
    }

    pub(super) fn canvas(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        let base = self.canvas_color();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(base))
            .show(root, |ui| {
                let (rect, response) = ui
                    .allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                self.canvas_rect = rect;

                if self.decoded.is_some() {
                    if response.dragged() {
                        self.view.pan += response.drag_delta();
                        self.view.fit = false;
                    }
                    if response.hovered() {
                        let (scroll, pinch, pointer) = ctx.input(|i| {
                            (
                                i.smooth_scroll_delta.y,
                                i.zoom_delta(),
                                i.pointer.hover_pos().unwrap_or(rect.center()),
                            )
                        });
                        if (pinch - 1.).abs() > 0.0001 {
                            self.view.zoom_at(pinch, pointer, rect.center());
                        } else if scroll.abs() > 0.01 {
                            self.view.zoom_at(
                                (scroll * 0.0025).exp(),
                                pointer,
                                rect.center(),
                            );
                        }
                        if response.dragged() {
                            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                        } else {
                            ctx.set_cursor_icon(egui::CursorIcon::Grab);
                        }
                    }
                    if response.double_clicked() {
                        self.view.reset();
                    }

                    let size = self
                        .decoded
                        .as_ref()
                        .map(|d| vec2(d.size[0] as f32, d.size[1] as f32))
                        .unwrap_or(Vec2::splat(1.));
                    if self.view.fit {
                        self.view.zoom = self.view.fit_zoom(
                            rect.size(),
                            size,
                            self.settings.fit_small,
                        );
                        // Small images should not be enlarged on high-DPI displays
                        if !self.settings.fit_small {
                            self.view.zoom =
                                self.view.zoom.min(1.0 / ctx.pixels_per_point());
                        }
                        self.view.pan = Vec2::ZERO;
                    }

                    let image_rect = self.view.image_rect(size, rect);
                    let painter = ui.painter().with_clip_rect(rect);
                    self.paint_backdrop(&painter, image_rect, base);
                    if let Some(texture) = &self.preview_texture {
                        self.paint_source(
                            &painter,
                            texture.id(),
                            Rect::from_min_size(Pos2::ZERO, size),
                            size,
                            rect,
                        );
                    }
                    self.paint_tiles(&painter, ctx, size, rect, base);
                    response.context_menu(|ui| {
                        self.context_menu(ui, ctx);
                    });
                } else if self.loading {
                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(Rect::from_center_size(
                            rect.center(),
                            vec2(250., 90.),
                        )),
                        |ui| {
                            ui.vertical_centered(|ui| {
                                ui.spinner();
                                ui.add_space(12.);
                                ui.label(
                                    RichText::new("Opening image…")
                                        .color(self.colors.muted),
                                );
                            });
                        },
                    );
                } else if let Some(error) = self.load_error.clone() {
                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(Rect::from_center_size(
                            rect.center(),
                            vec2(rect.width().min(560.) - 40., 240.),
                        )),
                        |ui| {
                            ui.vertical_centered(|ui| {
                                icons::logo(ui, 40., self.colors.muted);
                                ui.add_space(14.);
                                ui.heading("This image could not be opened");
                                ui.add_space(8.);
                                ui.label(RichText::new(error).color(self.colors.muted));
                                ui.add_space(16.);
                                ui.horizontal(|ui| {
                                    if ui.button("Open another image").clicked() {
                                        self.pick(false);
                                    }
                                    if self.files.len() > 1
                                        && ui.button("Next image").clicked()
                                    {
                                        self.navigate(1, ctx);
                                    }
                                });
                            });
                        },
                    );
                } else {
                    self.welcome(ui, rect);
                }

                if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
                    painter_drop(ui, rect, self.colors);
                }
            });
    }

    fn paint_backdrop(&self, painter: &egui::Painter, image_rect: Rect, base: Color32) {
        if !self.settings.checkerboard {
            return;
        }

        let visible = image_rect.intersect(painter.clip_rect());
        if !visible.is_positive() {
            return;
        }

        let square = self.settings.checker_size;
        let dark =
            u32::from(base.r()) + u32::from(base.g()) + u32::from(base.b()) < 384;
        let alternate: Color32 = egui::lerp(
            egui::Rgba::from(base)..=egui::Rgba::from(if dark {
                Color32::WHITE
            } else {
                Color32::BLACK
            }),
            self.settings.checker_contrast,
        )
        .into();

        let x0 = ((visible.left() - image_rect.left()) / square).floor() as i32;
        let y0 = ((visible.top() - image_rect.top()) / square).floor() as i32;
        let x1 = ((visible.right() - image_rect.left()) / square).ceil() as i32;
        let y1 = ((visible.bottom() - image_rect.top()) / square).ceil() as i32;

        let painter = painter.with_clip_rect(visible);
        for y in y0..y1 {
            for x in x0..x1 {
                if (x + y) % 2 == 0 {
                    painter.rect_filled(
                        Rect::from_min_size(
                            image_rect.min + vec2(x as f32 * square, y as f32 * square),
                            Vec2::splat(square),
                        ),
                        0.,
                        alternate,
                    );
                }
            }
        }
    }

    fn paint_source(
        &self,
        painter: &egui::Painter,
        texture: egui::TextureId,
        source: Rect,
        size: Vec2,
        canvas: Rect,
    ) {
        let points = [
            source.left_top(),
            source.right_top(),
            source.right_bottom(),
            source.left_bottom(),
        ]
        .map(|p| self.view.image_to_screen(p.to_vec2(), size, canvas));
        icons::uv_quad(painter, texture, points);
    }

    fn paint_tiles(
        &mut self,
        painter: &egui::Painter,
        ctx: &egui::Context,
        size: Vec2,
        canvas: Rect,
        base: Color32,
    ) {
        let Some(decoded) = self.decoded.as_ref() else {
            return;
        };
        if decoded.frames.len() > 1 && self.playing {
            return;
        }
        let Some(frame) = decoded.frames.get(self.frame_index) else {
            return;
        };
        if frame.preview.size == [decoded.size[0] as usize, decoded.size[1] as usize] {
            return;
        }

        let scale = self.view.zoom * ctx.pixels_per_point();
        let preview_scale = frame.preview.size[0] as f32 / size.x;
        if scale <= preview_scale * 1.02 {
            return;
        }

        // Choose the coarsest tile level that still supplies a sample per screen pixel
        let level = (1.0 / scale).log2().floor().clamp(0.0, 7.0) as u8;
        let span = loader::TILE_SIZE * (1_u32 << level);
        let visible = self.view.visible_source(size, canvas);
        if !visible.is_positive() {
            return;
        }

        let x0 = (visible.min.x.max(0.) as u32) / span;
        let y0 = (visible.min.y.max(0.) as u32) / span;
        let x1 = ((visible.max.x.ceil() as u32).div_ceil(span))
            .min(decoded.size[0].div_ceil(span));
        let y1 = ((visible.max.y.ceil() as u32).div_ceil(span))
            .min(decoded.size[1].div_ceil(span));

        let source_pixels = frame.pixels.clone();
        let mut keys = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                keys.push(TileKey {
                    generation: self.generation,
                    frame: self.frame_index,
                    level,
                    x,
                    y,
                });
            }
        }

        // Resolve the center first so detail appears where the user is looking
        keys.sort_by_key(|key| {
            let dx = (key.x * span + span / 2) as f32 - visible.center().x;
            let dy = (key.y * span + span / 2) as f32 - visible.center().y;
            (dx * dx + dy * dy) as u64
        });

        let image_rect = self.view.image_rect(size, canvas);
        let mut requests = 0;
        for key in keys {
            if let Some(tile) = self.tiles.get_mut(&key) {
                tile.touched = self.paint_counter;
                let texture_id = tile.texture.id();
                let source = tile.source;
                let core = Rect::from_min_max(
                    pos2((key.x * span) as f32, (key.y * span) as f32),
                    pos2(
                        ((key.x + 1) * span).min(size.x as u32) as f32,
                        ((key.y + 1) * span).min(size.y as u32) as f32,
                    ),
                );
                let core_points = [
                    core.left_top(),
                    core.right_top(),
                    core.right_bottom(),
                    core.left_bottom(),
                ]
                .map(|p| self.view.image_to_screen(p.to_vec2(), size, canvas));
                let clip = Rect::from_points(&core_points).intersect(canvas);
                let painter = painter.with_clip_rect(clip);

                // Clear the preview here so translucent pixels are not blended twice
                painter.rect_filled(clip, 0., base);
                self.paint_backdrop(&painter, image_rect, base);
                self.paint_source(&painter, texture_id, source, size, canvas);
            } else if requests < 4
                && !self.pending_tiles.contains(&key)
                && self
                    .tile_worker
                    .requests
                    .try_send(TileRequest {
                        key,
                        pixels: source_pixels.clone(),
                    })
                    .is_ok()
            {
                self.pending_tiles.insert(key);
                requests += 1;
            }
        }

        if requests > 0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let width = rect.width().min(480.);
        let height = if self.settings.recent.is_empty() {
            285.
        } else {
            425.
        };
        let area =
            Rect::from_center_size(rect.center() - vec2(0., 10.), vec2(width, height));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            ui.vertical_centered(|ui| {
                icons::logo(ui, 58., self.colors.accent);
                ui.add_space(18.);
                ui.label(RichText::new("flowa").size(34.));
                ui.add_space(8.);
                ui.label(
                    RichText::new("An open space for your images.")
                        .size(16.)
                        .color(self.colors.muted),
                );
                ui.add_space(24.);
                ui.horizontal(|ui| {
                    ui.add_space((width - 308.).max(0.) / 2.);
                    if ui
                        .add_sized(
                            [146., 42.],
                            egui::Button::new("Open image").fill(self.colors.raised),
                        )
                        .clicked()
                    {
                        self.pick(false);
                    }
                    if ui
                        .add_sized([146., 42.], egui::Button::new("Open folder"))
                        .clicked()
                    {
                        self.pick(true);
                    }
                });
                ui.add_space(14.);
                ui.label(
                    RichText::new("or drop an image or folder anywhere")
                        .size(13.)
                        .color(self.colors.muted),
                );
                if !self.settings.recent.is_empty() {
                    ui.add_space(30.);
                    ui.label(
                        RichText::new("RECENT IMAGES")
                            .size(11.)
                            .color(self.colors.muted),
                    );
                    ui.add_space(8.);
                    let mut open = None;
                    for path in self.settings.recent.iter().take(3) {
                        let name =
                            path.file_name().unwrap_or_default().to_string_lossy();
                        if ui
                            .add_sized(
                                [width.min(360.), 30.],
                                egui::Button::new(name).frame(false),
                            )
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            open = Some(path.clone());
                        }
                    }
                    if let Some(path) = open {
                        self.open_path(path, ui.ctx());
                    }
                }
            });
        });
    }
}

fn painter_drop(ui: &egui::Ui, rect: Rect, c: Colors) {
    ui.painter()
        .rect_filled(rect, 0., c.canvas.gamma_multiply(0.94));
    ui.painter().rect_stroke(
        rect.shrink(16.),
        12.,
        Stroke::new(2., c.accent),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to open",
        egui::FontId::proportional(26.),
        c.text,
    );
}
