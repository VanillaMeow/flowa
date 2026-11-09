//! Reversible image-to-screen transforms for flips, quarter turns, zoom, and pan

use egui::{Pos2, Rect, Vec2, vec2};

const MIN_ZOOM: f32 = 0.001;
pub const MAX_ZOOM: f32 = 128.0;

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

    /// The zoom at which one image pixel covers one physical screen pixel
    ///
    /// Zoom is measured in logical points, which hold `pixels_per_point` pixels each
    pub fn actual_pixels_zoom(pixels_per_point: f32) -> f32 {
        1.0 / pixels_per_point
    }

    /// Physical screen pixels per image pixel
    pub fn pixel_scale(&self, pixels_per_point: f32) -> f32 {
        self.zoom * pixels_per_point
    }

    pub fn is_actual_pixels(&self, pixels_per_point: f32) -> bool {
        !self.fit && (self.pixel_scale(pixels_per_point) - 1.0).abs() < 0.001
    }

    /// Sets a fixed zoom from physical screen pixels per image pixel
    pub fn set_pixel_scale(&mut self, scale: f32, pixels_per_point: f32) {
        self.fit = false;
        self.zoom = (scale / pixels_per_point).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn show_actual_pixels(&mut self, pixels_per_point: f32) {
        self.set_pixel_scale(1.0, pixels_per_point);
        self.pan = Vec2::ZERO;
    }

    /// Centers an image of `size` as large as fits in `area`, up to `max_zoom`
    pub fn fit_within(&mut self, area: Vec2, size: Vec2, max_zoom: f32) {
        let displayed = self.displayed_size(size);
        let fit = (area.x.max(1.0) / displayed.x).min(area.y.max(1.0) / displayed.y);

        self.zoom = fit.min(max_zoom).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = Vec2::ZERO;
    }

    pub fn reset(&mut self) {
        self.fit = true;
        self.pan = Vec2::ZERO;
    }

    pub fn rotate_clockwise(&mut self) {
        self.rotation = (self.rotation + 1) % 4;
    }

    pub fn rotate_counterclockwise(&mut self) {
        self.rotation = (self.rotation + 3) % 4;
    }

    pub fn flip_horizontal(&mut self) {
        self.flip_x = !self.flip_x;
    }

    pub fn flip_vertical(&mut self) {
        self.flip_y = !self.flip_y;
    }

    pub fn zoom_at(&mut self, factor: f32, pointer: Pos2, center: Pos2) {
        let previous_zoom = self.zoom;
        self.zoom = (previous_zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);

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
