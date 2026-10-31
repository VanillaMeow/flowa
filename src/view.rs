use egui::{Pos2, Rect, Vec2, vec2};

#[derive(Debug, Clone)]
pub struct View {
    pub zoom: f32,
    pub pan: Vec2,
    pub fit: bool,
    pub rotation: u8,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit: true,
            rotation: 0,
            flip_x: false,
            flip_y: false,
        }
    }
}

impl View {
    pub fn displayed_size(&self, size: Vec2) -> Vec2 {
        if self.rotation % 2 == 1 {
            vec2(size.y, size.x)
        } else {
            size
        }
    }

    pub fn fit_zoom(&self, canvas: Vec2, size: Vec2, enlarge: bool) -> f32 {
        let displayed_size = self.displayed_size(size);
        let fit_zoom = ((canvas.x - 48.0).max(1.0) / displayed_size.x)
            .min((canvas.y - 48.0).max(1.0) / displayed_size.y);
        if enlarge { fit_zoom } else { fit_zoom.min(1.0) }.clamp(0.001, 128.0)
    }

    pub fn reset(&mut self) {
        self.fit = true;
        self.pan = Vec2::ZERO;
    }

    pub fn zoom_at(&mut self, factor: f32, pointer: Pos2, center: Pos2) {
        let previous_zoom = self.zoom;
        self.zoom = (previous_zoom * factor).clamp(0.001, 128.0);

        // Keep the same image point under the cursor as the scale changes
        let pointer_offset = pointer - center - self.pan;
        self.pan += pointer_offset * (1.0 - self.zoom / previous_zoom);
        self.fit = false;
    }

    pub fn image_to_screen(&self, point: Vec2, size: Vec2, canvas: Rect) -> Pos2 {
        let mut offset = point - size / 2.0;

        if self.flip_x {
            offset.x = -offset.x;
        }
        if self.flip_y {
            offset.y = -offset.y;
        }

        offset = match self.rotation % 4 {
            1 => vec2(-offset.y, offset.x),
            2 => -offset,
            3 => vec2(offset.y, -offset.x),
            _ => offset,
        };

        canvas.center() + self.pan + offset * self.zoom
    }

    pub fn screen_to_image(&self, point: Pos2, size: Vec2, canvas: Rect) -> Vec2 {
        // Reverse the transform order used by image_to_screen
        let offset = (point - canvas.center() - self.pan) / self.zoom;
        let mut offset = match self.rotation % 4 {
            1 => vec2(offset.y, -offset.x),
            2 => -offset,
            3 => vec2(-offset.y, offset.x),
            _ => offset,
        };

        if self.flip_x {
            offset.x = -offset.x;
        }
        if self.flip_y {
            offset.y = -offset.y;
        }

        offset + size / 2.0
    }

    pub fn visible_source(&self, size: Vec2, canvas: Rect) -> Rect {
        let points = [
            canvas.left_top(),
            canvas.right_top(),
            canvas.right_bottom(),
            canvas.left_bottom(),
        ]
        .map(|p| self.screen_to_image(p, size, canvas).to_pos2());
        Rect::from_points(&points).intersect(Rect::from_min_size(Pos2::ZERO, size))
    }

    pub fn image_rect(&self, size: Vec2, canvas: Rect) -> Rect {
        Rect::from_center_size(
            canvas.center() + self.pan,
            self.displayed_size(size) * self.zoom,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_image_point_under_the_cursor() {
        let mut v = View::default();
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let size = vec2(1600.0, 1000.0);
        let cursor = egui::pos2(530.0, 380.0);
        let before = v.screen_to_image(cursor, size, rect);
        v.zoom_at(2.7, cursor, rect.center());
        let after = v.screen_to_image(cursor, size, rect);
        assert!((before - after).length() < 0.001);
    }

    #[test]
    fn transforms_round_trip_after_rotation_and_flips() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let size = vec2(1200.0, 900.0);
        let p = vec2(107.0, 618.0);
        for rotation in 0..4 {
            for flip_x in [false, true] {
                for flip_y in [false, true] {
                    let v = View {
                        rotation,
                        flip_x,
                        flip_y,
                        ..Default::default()
                    };
                    assert!(
                        (p - v.screen_to_image(
                            v.image_to_screen(p, size, rect),
                            size,
                            rect
                        ))
                        .length()
                            < 0.001
                    );
                }
            }
        }
    }
}
