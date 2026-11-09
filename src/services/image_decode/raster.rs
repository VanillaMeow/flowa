//! Still and animated raster formats decoded through the image crate

use std::{fs::File, io::BufReader, path::Path, time::Duration};

use anyhow::{Result, ensure};
use egui::ColorImage;
use image::{
    AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage,
    codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder},
    metadata::Orientation,
};

use super::{
    Budget, Decoded, PREVIEW_EDGE, THUMBNAIL_SIZE, color::ColorConversion, color_image,
    make_frame, resize,
};

const MAX_ANIMATION_FRAMES: usize = 2000;

type FileReader = BufReader<File>;

/// Memory needed to preload one image, estimated from its header
#[derive(Clone, Copy, Debug)]
pub struct PreloadEstimate {
    /// Pixels, preview, and thumbnail kept in the cache after decoding
    pub retained: u64,
    /// Peak memory while decoding, including conversion buffers
    pub working: u64,
}

impl PreloadEstimate {
    fn new(width: u32, height: u32, source_bytes: u64) -> Self {
        let pixels = u64::from(width) * u64::from(height) * 4;
        let preview = u64::from(width.min(PREVIEW_EDGE))
            * u64::from(height.min(PREVIEW_EDGE))
            * 4;
        let thumbnail = u64::from(width.min(THUMBNAIL_SIZE[0]))
            * u64::from(height.min(THUMBNAIL_SIZE[1]))
            * 4;
        let retained = pixels.saturating_add(preview).saturating_add(thumbnail);

        // Decoding holds the source buffer, its RGBA conversion, and an oriented copy at once
        let working = source_bytes
            .max(pixels)
            .saturating_mul(3)
            .saturating_add(preview)
            .saturating_add(thumbnail);

        Self { retained, working }
    }
}

/// One raster file, decoded within a memory budget
pub(super) struct RasterFile<'a> {
    path: &'a Path,
    budget: Budget,
}

impl<'a> RasterFile<'a> {
    pub fn new(path: &'a Path, budget: Budget) -> Self {
        Self { path, budget }
    }

    /// Decodes every frame of animated GIF, WebP, and PNG files, and stills otherwise
    pub fn decode(&self) -> Result<Decoded> {
        let reader = self.reader()?;

        match reader.format() {
            Some(ImageFormat::Gif) => {
                let decoder = self.gif()?;
                let (width, height) = decoder.dimensions();
                self.budget.check_pixels(width, height)?;

                self.animation(decoder.into_frames(), ColorConversion::new(None))
            }
            Some(ImageFormat::WebP) => {
                let mut decoder = self.webp()?;

                if !decoder.has_animation() {
                    return self.still(reader.into_decoder()?);
                }

                let conversion =
                    ColorConversion::new(decoder.icc_profile()?.as_deref());
                self.animation(decoder.into_frames(), conversion)
            }
            Some(ImageFormat::Png) => {
                let mut decoder = self.png()?;

                if !decoder.is_apng()? {
                    return self.still(decoder);
                }

                let conversion =
                    ColorConversion::new(decoder.icc_profile()?.as_deref());
                self.animation(decoder.apng()?.into_frames(), conversion)
            }
            _ => self.still(reader.into_decoder()?),
        }
    }

    /// Decodes the first frame straight to filmstrip size, converting only the small copy
    pub fn thumbnail(&self) -> Result<ColorImage> {
        let (pixels, conversion) = self.upright(self.reader()?.into_decoder()?)?;
        let [width, height] = resize::fit_within(
            pixels.width(),
            pixels.height(),
            THUMBNAIL_SIZE[0],
            THUMBNAIL_SIZE[1],
        );
        let mut small = resize::downscale(&pixels, None, width, height)?;

        conversion.apply(&mut small);
        Ok(color_image(&small))
    }

    /// Only static JPEG, PNG, WebP, BMP, and QOI images qualify, since their headers
    /// report reliable sizes and decoding them has no unbounded animation cost
    pub fn preload_estimate(&self) -> Result<Option<PreloadEstimate>> {
        let reader = self.reader()?;

        let (width, height, source_bytes) = match reader.format() {
            Some(ImageFormat::Png) => {
                let decoder = self.png()?;

                if decoder.is_apng()? {
                    return Ok(None);
                }

                let (width, height) = decoder.dimensions();
                (width, height, decoder.total_bytes())
            }
            Some(ImageFormat::WebP) => {
                let decoder = self.webp()?;

                if decoder.has_animation() {
                    return Ok(None);
                }

                let (width, height) = decoder.dimensions();
                (width, height, decoder.total_bytes())
            }
            Some(ImageFormat::Jpeg | ImageFormat::Bmp | ImageFormat::Qoi) => {
                let decoder = reader.into_decoder()?;
                let (width, height) = decoder.dimensions();
                (width, height, decoder.total_bytes())
            }
            _ => return Ok(None),
        };

        if width == 0 || height == 0 {
            return Ok(None);
        }

        Ok(Some(PreloadEstimate::new(width, height, source_bytes)))
    }

    fn still(&self, decoder: impl ImageDecoder) -> Result<Decoded> {
        let (mut pixels, conversion) = self.upright(decoder)?;
        conversion.apply(&mut pixels);

        Ok(Decoded {
            frames: vec![make_frame(pixels, Duration::ZERO)?],
            vector: None,
            color: conversion.label,
        })
    }

    /// Decodes upright RGBA pixels and the conversion that maps them to sRGB
    fn upright(
        &self,
        mut decoder: impl ImageDecoder,
    ) -> Result<(RgbaImage, ColorConversion)> {
        decoder.set_limits(self.budget.limits())?;
        let (width, height) = decoder.dimensions();
        self.budget.check_pixels(width, height)?;

        let conversion =
            ColorConversion::new(decoder.icc_profile().ok().flatten().as_deref());
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let mut image = DynamicImage::from_decoder(decoder)?;
        image.apply_orientation(orientation);

        Ok((image.into_rgba8(), conversion))
    }

    fn animation(
        &self,
        frames: image::Frames<'_>,
        conversion: ColorConversion,
    ) -> Result<Decoded> {
        let mut decoded = Vec::new();
        let mut used = 0_u64;

        for frame in frames {
            ensure!(
                decoded.len() < MAX_ANIMATION_FRAMES,
                "Animation exceeds the {MAX_ANIMATION_FRAMES}-frame limit"
            );

            let frame = frame?;
            let (num, den) = frame.delay().numer_denom_ms();
            let delay = Duration::from_secs_f64(
                (f64::from(num) / f64::from(den) / 1000.0).max(0.02),
            );
            let mut pixels = frame.into_buffer();
            conversion.apply(&mut pixels);

            // Account for both source pixels and the preview retained for each frame
            let preview = u64::from(pixels.width().min(PREVIEW_EDGE))
                * u64::from(pixels.height().min(PREVIEW_EDGE))
                * 4;
            used = used
                .saturating_add(pixels.as_raw().len() as u64)
                .saturating_add(preview);
            ensure!(
                self.budget.allows(used),
                "This animation exceeds the image memory limit. Increase it in Preferences."
            );

            decoded.push(make_frame(pixels, delay)?);
        }

        ensure!(!decoded.is_empty(), "The animation contains no frames");

        Ok(Decoded {
            frames: decoded,
            vector: None,
            color: conversion.label,
        })
    }

    fn file(&self) -> Result<FileReader> {
        Ok(BufReader::new(File::open(self.path)?))
    }

    fn reader(&self) -> Result<ImageReader<FileReader>> {
        let mut reader = ImageReader::open(self.path)?.with_guessed_format()?;
        reader.limits(self.budget.limits());

        Ok(reader)
    }

    fn gif(&self) -> Result<GifDecoder<FileReader>> {
        let mut decoder = GifDecoder::new(self.file()?)?;
        decoder.set_limits(self.budget.limits())?;

        Ok(decoder)
    }

    fn webp(&self) -> Result<WebPDecoder<FileReader>> {
        let mut decoder = WebPDecoder::new(self.file()?)?;
        decoder.set_limits(self.budget.limits())?;

        Ok(decoder)
    }

    fn png(&self) -> Result<PngDecoder<FileReader>> {
        Ok(PngDecoder::with_limits(self.file()?, self.budget.limits())?)
    }
}
