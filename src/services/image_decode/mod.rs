//! Image decoding, color conversion, previews, and export

mod color;
mod export;
mod raster;
mod vector;

use std::{fs::File, io::BufReader, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use egui::ColorImage;
use image::RgbaImage;

pub use export::{apply_view, export};
pub use raster::PreloadEstimate;

use crate::{
    utils::{
        size::{self, rgba_bytes},
        units::MIB,
    },
    viewer::{
        image_session::{DecodedFrame, DecodedImage},
        vector::VectorImage,
    },
};

use super::{image_workers::THUMBNAIL_SIZE, resize};
use raster::RasterFile;
use vector::VectorFile;

const PREVIEW_EDGE: u32 = 2048;

/// Decodes the full image, its preview, and its thumbnail within `budget` bytes
pub fn decode(path: &Path, budget: u64) -> Result<DecodedImage> {
    ImageFile::new(path, budget).decode()?.into_image(path)
}

/// Decodes a filmstrip thumbnail within `budget` bytes
pub fn thumbnail(path: &Path, budget: u64) -> Result<ColorImage> {
    ImageFile::new(path, budget).thumbnail()
}

/// Estimates memory for speculative decoding, or `None` for formats that are never preloaded
pub fn preload_estimate(path: &Path, budget: u64) -> Result<Option<PreloadEstimate>> {
    match ImageFile::new(path, budget) {
        ImageFile::Raster(file) => file.preload_estimate(),
        ImageFile::Vector(_) => Ok(None),
    }
}

pub fn color_image(pixels: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        pixels.as_raw(),
    )
}

/// Converts pixels for the GPU, shrinking them first when they exceed `bounds`
pub fn color_image_within(pixels: &RgbaImage, bounds: [u32; 2]) -> Result<ColorImage> {
    let full = pixels.dimensions().into();
    let fitted = size::shrink_to_fit(full, bounds);

    if fitted == full {
        return Ok(color_image(pixels));
    }

    Ok(color_image(&resize::downscale(pixels, None, fitted)?))
}

/// A file chosen by format, with the decoder that reads it
enum ImageFile<'a> {
    Raster(RasterFile<'a>),
    Vector(VectorFile<'a>),
}

impl<'a> ImageFile<'a> {
    fn new(path: &'a Path, budget: u64) -> Self {
        let budget = Budget(budget);

        if VectorFile::matches(path) {
            Self::Vector(VectorFile::new(path, budget))
        } else {
            Self::Raster(RasterFile::new(path, budget))
        }
    }

    fn decode(&self) -> Result<Decoded> {
        match self {
            Self::Raster(file) => file.decode(),
            Self::Vector(file) => file.decode(),
        }
    }

    fn thumbnail(&self) -> Result<ColorImage> {
        match self {
            Self::Raster(file) => file.thumbnail(),
            Self::Vector(file) => file.thumbnail(),
        }
    }
}

/// The memory, in bytes, that decoding one image may use
#[derive(Clone, Copy)]
struct Budget(u64);

impl Budget {
    fn limits(self) -> image::Limits {
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(self.0);
        limits.max_image_width = Some(131_072);
        limits.max_image_height = Some(131_072);
        limits
    }

    fn allows(self, bytes: u64) -> bool {
        bytes <= self.0
    }

    /// Fails when the RGBA pixels of an image this size would exceed the budget
    fn check_pixels(self, size: [u32; 2]) -> Result<()> {
        ensure!(size[0] > 0 && size[1] > 0, "The image has no pixels");

        let bytes = rgba_bytes(size);
        ensure!(
            self.allows(bytes),
            "This image needs at least {} MiB of decoded memory. Increase the image memory limit in Preferences.",
            (bytes / MIB) + 1
        );

        Ok(())
    }
}

/// What a format-specific decoder produces, before the shared details are added
struct Decoded {
    frames: Vec<DecodedFrame>,
    vector: Option<Arc<VectorImage>>,
    color: &'static str,
}

impl Decoded {
    fn into_image(self, path: &Path) -> Result<DecodedImage> {
        let first = self
            .frames
            .first()
            .context("The image contains no frames")?;
        let size = first.pixels.dimensions().into();
        let thumbnail = thumbnail_from_preview(&first.preview)?;

        Ok(DecodedImage {
            size,
            thumbnail,
            frames: self.frames,
            vector: self.vector,
            format: format_name(path),
            file_size: path.metadata()?.len(),
            color: self.color.to_owned(),
            metadata: exif_metadata(path),
        })
    }
}

/// Pairs decoded pixels with a downscaled preview for the GPU
fn make_frame(pixels: RgbaImage, delay: Duration) -> Result<DecodedFrame> {
    let preview = color_image_within(&pixels, [PREVIEW_EDGE, PREVIEW_EDGE])?;

    Ok(DecodedFrame {
        pixels: Arc::new(pixels),
        preview,
        delay,
    })
}

/// Builds the filmstrip thumbnail from a preview, which is far smaller than the source
fn thumbnail_from_preview(preview: &ColorImage) -> Result<ColorImage> {
    let full = preview.size.map(|edge| edge as u32);
    let thumbnail_size = size::shrink_to_fit(full, THUMBNAIL_SIZE);

    if thumbnail_size == full {
        return Ok(preview.clone());
    }

    resize::downscale_premultiplied(preview, thumbnail_size)
}

fn format_name(path: &Path) -> String {
    match path.extension() {
        Some(ext) if !ext.is_empty() => ext.to_string_lossy().to_ascii_uppercase(),
        _ => "Image".to_owned(),
    }
}

/// Reads photographic EXIF fields; metadata never prevents an image from opening
fn exif_metadata(path: &Path) -> Vec<(String, String)> {
    const TAGS: [exif::Tag; 8] = [
        exif::Tag::Make,
        exif::Tag::Model,
        exif::Tag::DateTimeOriginal,
        exif::Tag::ExposureTime,
        exif::Tag::FNumber,
        exif::Tag::PhotographicSensitivity,
        exif::Tag::FocalLength,
        exif::Tag::LensModel,
    ];

    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file))
    else {
        return Vec::new();
    };

    TAGS.into_iter()
        .filter_map(|tag| {
            let field = exif.get_field(tag, exif::In::PRIMARY)?;
            let value = field.display_value().with_unit(&exif).to_string();
            Some((tag.to_string(), value))
        })
        .collect()
}
