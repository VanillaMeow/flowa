//! SIMD box-filter downscaling for previews, thumbnails, and zoom tiles
//!
//! Box filtering averages every covered source pixel, like `image::imageops::thumbnail`,
//! but runs about three times faster on large photos

use anyhow::Result;
use egui::ColorImage;
use fast_image_resize::{
    FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
    images::{Image, ImageRef},
};
use image::RgbaImage;

/// A rectangle of source pixels
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

fn box_filter() -> ResizeOptions {
    ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Box))
}

/// Shrinks straight-alpha pixels, optionally from a region of the source
///
/// Colors are premultiplied while averaging so transparent pixels do not leave fringes
pub fn downscale(
    pixels: &RgbaImage,
    region: Option<Region>,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    let mut options = box_filter();

    if let Some(region) = region {
        options = options.crop(
            f64::from(region.x),
            f64::from(region.y),
            f64::from(region.width),
            f64::from(region.height),
        );
    }

    let mut output = RgbaImage::new(width, height);
    Resizer::new().resize(pixels, &mut output, &options)?;

    Ok(output)
}

/// Shrinks pixels that are already premultiplied, such as a decoded preview
pub fn downscale_premultiplied(
    image: &ColorImage,
    width: u32,
    height: u32,
) -> Result<ColorImage> {
    let source = ImageRef::new(
        u32::try_from(image.size[0])?,
        u32::try_from(image.size[1])?,
        image.as_raw(),
        PixelType::U8x4,
    )?;
    let mut output = Image::new(width, height, PixelType::U8x4);
    Resizer::new().resize(&source, &mut output, &box_filter().use_alpha(false))?;

    Ok(ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        output.buffer(),
    ))
}

/// The largest size within the maximum that keeps the aspect ratio, never enlarging
pub fn fit_within(
    width: u32,
    height: u32,
    max_width: u32,
    max_height: u32,
) -> [u32; 2] {
    if width <= max_width && height <= max_height {
        return [width, height];
    }

    let scale = (f64::from(max_width) / f64::from(width))
        .min(f64::from(max_height) / f64::from(height));

    [
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    ]
}
