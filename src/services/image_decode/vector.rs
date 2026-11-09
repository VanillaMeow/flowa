//! SVG documents that render sharply at any scale

use std::{
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use egui::Pos2;
use image::RgbaImage;
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg::{Options, Tree, fontdb},
};

use super::{Budget, Decoded, PREVIEW_EDGE, color_image};
use crate::{utils::units::MIB, viewer::image_session::DecodedFrame};

const MAX_SOURCE_BYTES: u64 = 32 * MIB;
/// The longest edge of the pixels kept for copying and exporting
const MAX_PIXEL_EDGE: f32 = 16384.0;

pub(super) fn is_svg(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz")
    })
}

/// A parsed SVG, measured in image pixels at its intrinsic size
pub struct VectorImage {
    tree: Tree,
    /// Image pixels per SVG unit, below one only for SVGs beyond the pixel limit
    units: f32,
    size: [u32; 2],
}

impl VectorImage {
    pub fn open(path: &Path) -> Result<Self> {
        let too_large = "SVG source exceeds the 32 MiB limit";
        ensure!(path.metadata()?.len() <= MAX_SOURCE_BYTES, too_large);

        // The file can grow between the size check and the read
        let data = std::fs::read(path)?;
        ensure!(data.len() as u64 <= MAX_SOURCE_BYTES, too_large);

        let tree = Tree::from_data(&data, &options(path, &data))?;
        let intrinsic = tree.size();
        let units =
            (MAX_PIXEL_EDGE / intrinsic.width().max(intrinsic.height())).min(1.0);
        let size = [intrinsic.width(), intrinsic.height()]
            .map(|edge| (edge * units).ceil().max(1.0) as u32);

        Ok(Self { tree, units, size })
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
        let [width, height] = self.size.map(|edge| edge as f32);
        let scale = (bounds[0] as f32 / width).min(bounds[1] as f32 / height);
        let size = [width, height].map(|edge| ((edge * scale).round() as u32).max(1));

        self.render(Pos2::ZERO, scale, size)
    }

    /// Keeps pixels at the intrinsic size for copying and exporting, while the tree
    /// serves the preview and zoomed tiles
    pub(super) fn decode(self, budget: Budget) -> Result<Decoded> {
        budget.check_pixels(self.size[0], self.size[1])?;

        let pixels = self.render(Pos2::ZERO, 1.0, self.size)?;
        let preview = self.render_fitted([PREVIEW_EDGE, PREVIEW_EDGE])?;
        let frame = DecodedFrame {
            pixels: Arc::new(pixels),
            preview: color_image(&preview),
            delay: Duration::ZERO,
        };

        Ok(Decoded {
            frames: vec![frame],
            vector: Some(Arc::new(self)),
            color: "sRGB · vector SVG",
        })
    }
}

fn options(path: &Path, data: &[u8]) -> Options<'static> {
    let mut options = Options::default();

    // Plain SVGs without text can skip expensive system-font discovery
    let compressed = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svgz"));
    if compressed || data.windows(5).any(|window| window == b"<text") {
        options.fontdb = system_fonts();
    }

    // Allow embedded images, but never resolve files referenced by the SVG
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);

    options
}

fn system_fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();

    FONTS
        .get_or_init(|| {
            let mut fonts = fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone()
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
