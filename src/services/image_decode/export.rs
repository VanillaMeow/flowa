//! Writing transformed copies of decoded images

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use image::{DynamicImage, RgbaImage};

use crate::viewer::transform::View;

/// Applies the viewer's flips and quarter turns to a copy of the pixels
pub fn apply_view(pixels: &RgbaImage, view: &View) -> DynamicImage {
    let mut image = DynamicImage::ImageRgba8(pixels.clone());

    if view.flip_x {
        image = image.fliph();
    }

    if view.flip_y {
        image = image.flipv();
    }

    match view.rotation % 4 {
        1 => image.rotate90(),
        2 => image.rotate180(),
        3 => image.rotate270(),
        _ => image,
    }
}

pub fn export(pixels: &RgbaImage, path: &Path, view: &View) -> Result<()> {
    let format = image::ImageFormat::from_path(path)
        .context("Choose a .png, .jpg, .webp, .bmp, or .tiff extension")?;

    ensure!(
        matches!(
            format,
            image::ImageFormat::Png
                | image::ImageFormat::Jpeg
                | image::ImageFormat::WebP
                | image::ImageFormat::Bmp
                | image::ImageFormat::Tiff
        ),
        "Export supports PNG, JPEG, WebP, BMP, and TIFF"
    );

    let mut image = apply_view(pixels, view);

    if format == image::ImageFormat::Jpeg {
        image = DynamicImage::ImageRgb8(image.into_rgb8());
    }

    // Encode to a temporary sibling and replace only once encoding succeeds
    let parent = path.parent().context("Export path has no parent")?;
    let temp = parent.join(format!(".flowa-export-{}.tmp", std::process::id()));

    if let Err(err) = image.save_with_format(&temp, format) {
        let _ = std::fs::remove_file(&temp);
        bail!(err);
    }

    std::fs::rename(&temp, path)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
        .context("Could not finish export")
}
