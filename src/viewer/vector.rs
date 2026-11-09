//! Vector images that render sharply at any scale

use anyhow::{Context, Result};
use egui::Pos2;
use image::RgbaImage;
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg::Tree,
};

use crate::utils::size;

/// A parsed SVG, measured in image pixels
pub struct VectorImage {
    tree: Tree,
    /// Image pixels per SVG unit, below one only for SVGs larger than the edge limit
    units: f32,
    size: [u32; 2],
}

impl VectorImage {
    /// Measures `tree` at its intrinsic size, shrunk so no edge exceeds `max_edge`
    pub fn new(tree: Tree, max_edge: f32) -> Self {
        let intrinsic = tree.size();
        let units = (max_edge / intrinsic.width().max(intrinsic.height())).min(1.0);
        let size = [intrinsic.width(), intrinsic.height()]
            .map(|edge| (edge * units).ceil().max(1.0) as u32);

        Self { tree, units, size }
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Renders `size` output pixels from image position `origin`, at `scale` output
    /// pixels per image pixel
    pub fn render(
        &self,
        origin: Pos2,
        scale: f32,
        size: [u32; 2],
    ) -> Result<RgbaImage> {
        let [width, height] = size;
        let mut pixmap =
            Pixmap::new(width, height).context("Could not allocate SVG pixels")?;
        let units = self.units * scale;
        let transform = Transform::from_scale(units, units)
            .post_translate(-origin.x * scale, -origin.y * scale);

        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        unpremultiply(&mut pixmap);

        RgbaImage::from_raw(width, height, pixmap.take())
            .context("Invalid SVG pixel buffer")
    }

    /// Renders the whole image as large as fits within `bounds`, enlarging small SVGs
    pub fn render_fitted(&self, bounds: [u32; 2]) -> Result<RgbaImage> {
        let scale = size::fit_scale(self.size, bounds) as f32;

        self.render(Pos2::ZERO, scale, size::scale_to_fit(self.size, bounds))
    }
}

/// Tiny-skia premultiplies alpha; the viewer expects straight-alpha pixels
fn unpremultiply(pixmap: &mut Pixmap) {
    for rgba in pixmap.data_mut().as_chunks_mut::<4>().0 {
        let alpha = u32::from(rgba[3]);

        for channel in &mut rgba[..3] {
            *channel = (u32::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
}
