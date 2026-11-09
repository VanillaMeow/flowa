//! SVG files parsed into vector images

use std::{
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::{Result, ensure};
use egui::{ColorImage, Pos2};
use resvg::usvg::{Options, Tree, fontdb};

use super::{Budget, Decoded, PREVIEW_EDGE, THUMBNAIL_SIZE, color_image};
use crate::{
    utils::units::MIB,
    viewer::{image_session::DecodedFrame, vector::VectorImage},
};

const MAX_SOURCE_BYTES: u64 = 32 * MIB;
/// The longest edge of the pixels kept for copying and exporting
const MAX_PIXEL_EDGE: f32 = 16384.0;

/// One SVG file, decoded within a memory budget
pub(super) struct VectorFile<'a> {
    path: &'a Path,
    budget: Budget,
}

impl<'a> VectorFile<'a> {
    pub fn new(path: &'a Path, budget: Budget) -> Self {
        Self { path, budget }
    }

    pub fn matches(path: &Path) -> bool {
        path.extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz")
        })
    }

    /// Keeps pixels at the intrinsic size for copying and exporting, while the vector
    /// serves the preview and zoomed tiles
    pub fn decode(&self) -> Result<Decoded> {
        let vector = self.open()?;
        self.budget.check_pixels(vector.size())?;

        let pixels = vector.render(Pos2::ZERO, 1.0, vector.size())?;
        let preview = vector.render_fitted([PREVIEW_EDGE, PREVIEW_EDGE])?;
        let frame = DecodedFrame {
            pixels: Arc::new(pixels),
            preview: color_image(&preview),
            delay: Duration::ZERO,
        };

        Ok(Decoded {
            frames: vec![frame],
            vector: Some(Arc::new(vector)),
            color: "sRGB · vector SVG",
        })
    }

    /// Fills the filmstrip box regardless of the SVG's intrinsic size
    pub fn thumbnail(&self) -> Result<ColorImage> {
        Ok(color_image(&self.open()?.render_fitted(THUMBNAIL_SIZE)?))
    }

    fn open(&self) -> Result<VectorImage> {
        let too_large = "SVG source exceeds the 32 MiB limit";
        ensure!(self.path.metadata()?.len() <= MAX_SOURCE_BYTES, too_large);

        // The file can grow between the size check and the read
        let data = std::fs::read(self.path)?;
        ensure!(data.len() as u64 <= MAX_SOURCE_BYTES, too_large);

        let tree = Tree::from_data(&data, &self.options(&data))?;

        Ok(VectorImage::new(tree, MAX_PIXEL_EDGE))
    }

    fn options(&self, data: &[u8]) -> Options<'static> {
        let mut options = Options::default();

        // Plain SVGs without text can skip expensive system-font discovery
        let compressed = self
            .path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("svgz"));
        if compressed || data.windows(5).any(|window| window == b"<text") {
            options.fontdb = system_fonts();
        }

        // Allow embedded images, but never resolve files referenced by the SVG
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);

        options
    }
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
