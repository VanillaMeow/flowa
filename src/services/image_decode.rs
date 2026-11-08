//! Image decoding, color conversion, previews, and export

use std::{fs::File, io::BufReader, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use egui::ColorImage;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageReader, RgbaImage};

use crate::viewer::{
    image_session::{DecodedFrame, DecodedImage},
    transform::View,
};

use super::{image_workers::THUMBNAIL_SIZE, resize};
use crate::utils::units::MIB;

const PREVIEW_EDGE: u32 = 2048;
const MAX_ANIMATION_FRAMES: usize = 2000;
const UNTAGGED: &str = "sRGB / untagged";
const MAX_SVG_BYTES: u64 = 32 * MIB;

fn limits(budget: u64) -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(budget);
    limits.max_image_width = Some(131_072);
    limits.max_image_height = Some(131_072);
    limits
}

fn check_size(width: u32, height: u32, budget: u64) -> Result<()> {
    ensure!(width > 0 && height > 0, "The image has no pixels");

    let bytes = u64::from(width) * u64::from(height) * 4;
    ensure!(
        bytes <= budget,
        "This image needs at least {} MiB of decoded memory. Increase the image memory limit in Preferences.",
        (bytes / MIB) + 1
    );

    Ok(())
}

fn preview(pixels: &RgbaImage, edge: u32) -> Result<ColorImage> {
    let [width, height] =
        resize::fit_within(pixels.width(), pixels.height(), edge, edge);

    if [width, height] == [pixels.width(), pixels.height()] {
        return Ok(color_image(pixels));
    }

    Ok(color_image(&resize::downscale(
        pixels, None, width, height,
    )?))
}

/// Builds the filmstrip thumbnail from a preview, which is far smaller than the source
fn thumbnail_from_preview(preview: &ColorImage) -> Result<ColorImage> {
    let [width, height] = [preview.size[0] as u32, preview.size[1] as u32];
    let [thumbnail_width, thumbnail_height] =
        resize::fit_within(width, height, THUMBNAIL_SIZE[0], THUMBNAIL_SIZE[1]);

    if [thumbnail_width, thumbnail_height] == [width, height] {
        return Ok(preview.clone());
    }

    resize::downscale_premultiplied(preview, thumbnail_width, thumbnail_height)
}

pub fn color_image(pixels: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        pixels.as_raw(),
    )
}

/// Memory needed to preload one image, estimated from its header
#[derive(Clone, Copy, Debug)]
pub struct PreloadEstimate {
    /// Pixels, preview, and thumbnail kept in the cache after decoding
    pub retained: u64,
    /// Peak memory while decoding, including conversion buffers
    pub working: u64,
}

/// Estimates memory for speculative decoding, or `None` for formats that are never preloaded
///
/// Only static JPEG, PNG, WebP, BMP, and QOI images qualify, since their headers report
/// reliable sizes and decoding them has no unbounded animation or vector cost
pub fn preload_estimate(path: &Path, budget: u64) -> Result<Option<PreloadEstimate>> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(limits(budget));

    let (width, height, source_bytes) = match reader.format() {
        Some(image::ImageFormat::Png) => {
            let decoder = image::codecs::png::PngDecoder::with_limits(
                BufReader::new(File::open(path)?),
                limits(budget),
            )?;

            if decoder.is_apng()? {
                return Ok(None);
            }

            let (width, height) = decoder.dimensions();
            (width, height, decoder.total_bytes())
        }
        Some(image::ImageFormat::WebP) => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(BufReader::new(
                File::open(path)?,
            ))?;
            decoder.set_limits(limits(budget))?;

            if decoder.has_animation() {
                return Ok(None);
            }

            let (width, height) = decoder.dimensions();
            (width, height, decoder.total_bytes())
        }
        Some(
            image::ImageFormat::Jpeg
            | image::ImageFormat::Bmp
            | image::ImageFormat::Qoi,
        ) => {
            let decoder = reader.into_decoder()?;
            let (width, height) = decoder.dimensions();
            (width, height, decoder.total_bytes())
        }
        _ => return Ok(None),
    };

    if width == 0 || height == 0 {
        return Ok(None);
    }

    let pixels = u64::from(width) * u64::from(height) * 4;
    let preview =
        u64::from(width.min(PREVIEW_EDGE)) * u64::from(height.min(PREVIEW_EDGE)) * 4;
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

    Ok(Some(PreloadEstimate { retained, working }))
}

pub fn decode(path: &Path, budget: u64) -> Result<DecodedImage> {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let file_size = path.metadata()?.len();

    let (frames, color) = if matches!(ext.as_str(), "svg" | "svgz") {
        let rgba = render_svg(path, 16384, budget)?;
        let frame = make_frame(rgba, Duration::ZERO)?;
        (vec![frame], "sRGB · rasterized SVG".to_owned())
    } else {
        decode_raster(path, budget)?
    };

    let first = frames.first().context("The image contains no frames")?;
    let size = [first.pixels.width(), first.pixels.height()];
    let thumbnail = thumbnail_from_preview(&first.preview)?;
    let format = if ext.is_empty() {
        "Image".to_owned()
    } else {
        ext.to_ascii_uppercase()
    };
    Ok(DecodedImage {
        frames,
        thumbnail,
        size,
        format,
        file_size,
        color,
        metadata: exif_metadata(path),
    })
}

/// Decodes still and animated raster formats, returning the frames and a color label
fn decode_raster(path: &Path, budget: u64) -> Result<(Vec<DecodedFrame>, String)> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let format = reader.format();
    reader.limits(limits(budget));

    match format {
        Some(image::ImageFormat::Gif) => {
            let mut decoder =
                image::codecs::gif::GifDecoder::new(BufReader::new(File::open(path)?))?;
            decoder.set_limits(limits(budget))?;

            let (width, height) = decoder.dimensions();
            check_size(width, height, budget)?;

            let conversion = ColorConversion::new(None);
            let frames = animated_frames(decoder.into_frames(), &conversion, budget)?;
            Ok((frames, conversion.label.to_owned()))
        }
        Some(image::ImageFormat::WebP) => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(BufReader::new(
                File::open(path)?,
            ))?;
            decoder.set_limits(limits(budget))?;

            if !decoder.has_animation() {
                return static_frame(reader.into_decoder()?, budget);
            }

            let conversion = ColorConversion::new(decoder.icc_profile()?.as_deref());
            let frames = animated_frames(decoder.into_frames(), &conversion, budget)?;
            Ok((frames, conversion.label.to_owned()))
        }
        Some(image::ImageFormat::Png) => {
            let mut decoder = image::codecs::png::PngDecoder::with_limits(
                BufReader::new(File::open(path)?),
                limits(budget),
            )?;

            if !decoder.is_apng()? {
                return static_frame(decoder, budget);
            }

            let conversion = ColorConversion::new(decoder.icc_profile()?.as_deref());
            let frames =
                animated_frames(decoder.apng()?.into_frames(), &conversion, budget)?;
            Ok((frames, conversion.label.to_owned()))
        }
        _ => static_frame(reader.into_decoder()?, budget),
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

fn make_frame(pixels: RgbaImage, delay: Duration) -> Result<DecodedFrame> {
    let preview = preview(&pixels, PREVIEW_EDGE)?;

    Ok(DecodedFrame {
        pixels: Arc::new(pixels),
        preview,
        delay,
    })
}

fn static_frame(
    mut decoder: impl ImageDecoder,
    budget: u64,
) -> Result<(Vec<DecodedFrame>, String)> {
    decoder.set_limits(limits(budget))?;
    let (w, h) = decoder.dimensions();
    check_size(w, h, budget)?;
    let conversion =
        ColorConversion::new(decoder.icc_profile().ok().flatten().as_deref());
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);

    let mut rgba = image.into_rgba8();
    conversion.apply(&mut rgba);
    Ok((
        vec![make_frame(rgba, Duration::ZERO)?],
        conversion.label.to_owned(),
    ))
}

type SrgbTransform = lcms2::Transform<u8, u8>;

/// Converts embedded profiles to sRGB, or explains why pixels are shown unconverted
struct ColorConversion {
    transform: Option<SrgbTransform>,
    label: &'static str,
}

impl ColorConversion {
    fn new(icc: Option<&[u8]>) -> Self {
        match srgb_transform(icc) {
            Ok(transform) => Self {
                transform: Some(transform),
                label: "Embedded ICC → sRGB",
            },
            Err(label) => Self {
                transform: None,
                label,
            },
        }
    }

    fn apply(&self, pixels: &mut RgbaImage) {
        if let Some(transform) = &self.transform {
            transform.transform_in_place(pixels.as_mut());
        }
    }
}

fn srgb_transform(icc: Option<&[u8]>) -> Result<SrgbTransform, &'static str> {
    let icc = icc.ok_or(UNTAGGED)?;
    let profile = lcms2::Profile::new_icc(icc)
        .map_err(|_| "Invalid ICC profile · displayed as sRGB")?;

    if profile.color_space() != lcms2::ColorSpaceSignature::RgbData {
        return Err("Non-RGB ICC · displayed as sRGB");
    }

    lcms2::Transform::new_flags(
        &profile,
        lcms2::PixelFormat::RGBA_8,
        &lcms2::Profile::new_srgb(),
        lcms2::PixelFormat::RGBA_8,
        lcms2::Intent::RelativeColorimetric,
        lcms2::Flags::COPY_ALPHA,
    )
    .map_err(|_| "ICC conversion unavailable · displayed as sRGB")
}

fn animated_frames(
    frames: image::Frames<'_>,
    conversion: &ColorConversion,
    budget: u64,
) -> Result<Vec<DecodedFrame>> {
    let mut out = Vec::new();
    let mut used = 0_u64;

    for frame in frames {
        ensure!(
            out.len() < MAX_ANIMATION_FRAMES,
            "Animation exceeds the {MAX_ANIMATION_FRAMES}-frame limit"
        );
        let frame = frame?;
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = Duration::from_secs_f64(
            (f64::from(num) / f64::from(den) / 1000.0).max(0.02),
        );
        let mut buffer = frame.into_buffer();
        conversion.apply(&mut buffer);

        // Account for both source pixels and the preview retained for each frame
        used = used
            .saturating_add(buffer.as_raw().len() as u64)
            .saturating_add(
                u64::from(buffer.width().min(PREVIEW_EDGE))
                    * u64::from(buffer.height().min(PREVIEW_EDGE))
                    * 4,
            );
        ensure!(
            used <= budget,
            "This animation exceeds the image memory limit. Increase it in Preferences."
        );
        out.push(make_frame(buffer, delay)?);
    }
    ensure!(!out.is_empty(), "The animation contains no frames");
    Ok(out)
}

fn render_svg(path: &Path, edge: u32, budget: u64) -> Result<RgbaImage> {
    let too_large = "SVG source exceeds the 32 MiB limit";
    ensure!(path.metadata()?.len() <= MAX_SVG_BYTES, too_large);

    // The file can grow between the size check and the read
    let data = std::fs::read(path)?;
    ensure!(data.len() as u64 <= MAX_SVG_BYTES, too_large);
    let mut options = resvg::usvg::Options::default();

    // Plain SVGs without text can skip expensive system-font discovery
    if data.windows(5).any(|w| w == b"<text")
        || path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("svgz"))
    {
        static FONTS: std::sync::OnceLock<Arc<resvg::usvg::fontdb::Database>> =
            std::sync::OnceLock::new();
        options.fontdb = FONTS
            .get_or_init(|| {
                let mut db = resvg::usvg::fontdb::Database::new();
                db.load_system_fonts();
                Arc::new(db)
            })
            .clone();
    }

    // Allow embedded images, but never resolve files referenced by the SVG
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = resvg::usvg::Tree::from_data(&data, &options)?;
    let size = tree.size();
    let scale = (edge as f32 / size.width().max(size.height())).min(1.0);
    let w = (size.width() * scale).ceil().max(1.0) as u32;
    let h = (size.height() * scale).ceil().max(1.0) as u32;
    check_size(w, h, budget)?;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(w, h).context("Could not allocate SVG pixels")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // Tiny-skia premultiplies alpha; the viewer expects straight-alpha pixels
    for rgba in pixmap.data_mut().as_chunks_mut::<4>().0 {
        let alpha = u32::from(rgba[3]);
        for c in &mut rgba[..3] {
            *c = (u32::from(*c) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    RgbaImage::from_raw(w, h, pixmap.take()).context("Invalid SVG pixel buffer")
}

/// Decodes a filmstrip thumbnail within the user's image memory limit
pub fn thumbnail(path: &Path, budget: u64) -> Result<ColorImage> {
    if path.extension().is_some_and(|ext| {
        ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz")
    }) {
        return Ok(color_image(&render_svg(path, 256, budget)?));
    }

    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(limits(budget));

    let mut decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    check_size(w, h, budget)?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let conversion =
        ColorConversion::new(decoder.icc_profile().ok().flatten().as_deref());

    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let rgba = image.into_rgba8();
    let [width, height] = resize::fit_within(
        rgba.width(),
        rgba.height(),
        THUMBNAIL_SIZE[0],
        THUMBNAIL_SIZE[1],
    );
    let mut small = resize::downscale(&rgba, None, width, height)?;

    conversion.apply(&mut small);
    Ok(color_image(&small))
}

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
