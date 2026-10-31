//! Visible thumbnails and horizontal folder navigation

use super::*;

impl Flowa {
    pub(super) fn filmstrip(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();

        if self.files.is_empty() {
            return;
        }

        egui::Panel::bottom("filmstrip")
            .exact_size(116.)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.panel)
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(root, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("thumbnails")
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, viewport| {
                        let origin = ui.cursor().min;
                        let stride = 116.;
                        ui.set_min_size(vec2(self.files.len() as f32 * stride, 86.));
                        if self.scroll_to_current {
                            let selected = Rect::from_min_size(
                                origin + vec2(self.index as f32 * stride, 0.),
                                vec2(108., 86.),
                            );
                            ui.scroll_to_rect(selected, Some(egui::Align::Center));
                            self.scroll_to_current = false;
                        }

                        // Large folders only create widgets for thumbnails in view
                        let start = (viewport.min.x / stride).floor().max(0.) as usize;
                        let end = ((viewport.max.x / stride).ceil() as usize + 1)
                            .min(self.files.len());
                        let mut clicked = None;

                        for index in start..end {
                            let entry = &self.files[index];
                            let path = entry.path.clone();
                            let rect = Rect::from_min_size(
                                origin + vec2(index as f32 * stride, 0.),
                                vec2(108., 86.),
                            );
                            let response = ui.interact(
                                rect,
                                ui.scope_id().with(index),
                                Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    true,
                                    &entry.name,
                                )
                            });
                            let selected = index == self.index;
                            ui.painter().rect_filled(
                                rect,
                                7.,
                                if selected {
                                    self.colors.raised
                                } else {
                                    self.colors.canvas
                                },
                            );

                            if let Some(Some(texture)) = self.thumbnails.get(&path) {
                                let area = Rect::from_min_max(
                                    rect.min + vec2(5., 4.),
                                    rect.max - vec2(5., 22.),
                                );
                                let size = texture.size_vec2();
                                let scale =
                                    (area.width() / size.x).min(area.height() / size.y);
                                ui.painter().image(
                                    texture.id(),
                                    Rect::from_center_size(area.center(), size * scale),
                                    Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                                    Color32::WHITE,
                                );
                            } else {
                                icons::paint(
                                    ui.painter(),
                                    Icon::Image,
                                    rect.center() - vec2(0., 9.),
                                    22.,
                                    self.colors.muted,
                                );
                                if !self.thumbnails.contains_key(&path)
                                    && !self.thumb_pending.contains(&path)
                                    && self
                                        .thumbnail_worker
                                        .requests
                                        .try_send(ThumbnailRequest {
                                            path: path.clone(),
                                            generation: self.scan_serial,
                                        })
                                        .is_ok()
                                {
                                    self.thumb_pending.insert(path);
                                }
                            }

                            let text_rect = Rect::from_min_max(
                                pos2(rect.left() + 5., rect.bottom() - 19.),
                                rect.max - vec2(5., 1.),
                            );
                            let text = egui::WidgetText::from(
                                RichText::new(&entry.name).size(11.).color(
                                    if selected {
                                        self.colors.text
                                    } else {
                                        self.colors.muted
                                    },
                                ),
                            );
                            let galley = text.into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                text_rect.width(),
                                egui::TextStyle::Small,
                            );
                            ui.painter().galley(
                                text_rect.left_top(),
                                galley,
                                self.colors.text,
                            );
                            if selected || response.has_focus() || response.hovered() {
                                ui.painter().rect_stroke(
                                    rect,
                                    7.,
                                    Stroke::new(
                                        if selected { 1.5 } else { 1. },
                                        if selected || response.has_focus() {
                                            self.colors.accent
                                        } else {
                                            self.colors.border
                                        },
                                    ),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if response.clicked() {
                                clicked = Some(index);
                            }
                            response.on_hover_text(&entry.name);
                        }

                        // Change selection after releasing the borrowed file entry
                        if let Some(index) = clicked {
                            self.select(index, ctx);
                        }
                    });
            });
    }
}
