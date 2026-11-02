//! Image decoding, color conversion, previews, and export

use std::{fs::File, io::BufReader, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use egui::ColorImage;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageReader, RgbaImage};

use crate::viewer::image_session::{DecodedFrame, DecodedImage};

const PREVIEW_EDGE: u32 = 2048;
const MAX_FRAMES: usize = 2000;

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
        "This image needs at least {} MB of decoded memory. Increase the image memory limit in Settings.",
        (bytes / 1048576) + 1
    );

    Ok(())
}

fn preview(pixels: &RgbaImage, edge: u32) -> ColorImage {
    let longest = pixels.width().max(pixels.height());

    if longest <= edge {
        return color_image(pixels);
    }

    let width = (u64::from(pixels.width()) * u64::from(edge) / u64::from(longest))
        .max(1) as u32;
    let height = (u64::from(pixels.height()) * u64::from(edge) / u64::from(longest))
        .max(1) as u32;
    let resized = image::imageops::thumbnail(pixels, width, height);

    color_image(&resized)
}

pub fn color_image(pixels: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        pixels.as_raw(),
    )
}

pub fn preload_bytes(path: &Path, budget: u64) -> Result<Option<u64>> {
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

    // Leave room for conversion buffers as well as the retained pixels and previews
    let pixels = u64::from(width) * u64::from(height) * 4;
    let preview =
        u64::from(width.min(PREVIEW_EDGE)) * u64::from(height.min(PREVIEW_EDGE)) * 4;
    let thumbnail = u64::from(width.min(192)) * u64::from(height.min(192)) * 4;
    let reserved = source_bytes
        .max(pixels)
        .saturating_mul(3)
        .saturating_add(preview)
        .saturating_add(thumbnail);

    Ok((width > 0 && height > 0 && reserved <= budget).then_some(reserved))
}

pub fn decode(path: &Path, budget: u64) -> Result<DecodedImage> {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let file_size = path.metadata()?.len();
    let mut color = "sRGB / untagged".to_owned();
    let mut metadata = Vec::new();

    let frames = if matches!(ext.as_str(), "svg" | "svgz") {
        let rgba = render_svg(path, 16384, budget)?;
        color = "sRGB · rasterized SVG".to_owned();
        vec![make_frame(rgba, Duration::ZERO)]
    } else {
        let mut reader = ImageReader::open(path)?.with_guessed_format()?;
        let format = reader.format();
        reader.limits(limits(budget));

        match format {
            Some(image::ImageFormat::Gif) => {
                let mut decoder = image::codecs::gif::GifDecoder::new(BufReader::new(
                    File::open(path)?,
                ))?;
                decoder.set_limits(limits(budget))?;

                check_size(decoder.dimensions().0, decoder.dimensions().1, budget)?;

                animated_frames(decoder.into_frames(), budget)?
            }
            Some(image::ImageFormat::WebP) => {
                let mut decoder = image::codecs::webp::WebPDecoder::new(
                    BufReader::new(File::open(path)?),
                )?;
                decoder.set_limits(limits(budget))?;

                if decoder.has_animation() {
                    let icc = decoder.icc_profile()?;
                    let mut frames = animated_frames(decoder.into_frames(), budget)?;
                    for frame in &mut frames {
                        color =
                            apply_icc(Arc::make_mut(&mut frame.pixels), icc.as_deref());
                        frame.preview = preview(&frame.pixels, PREVIEW_EDGE);
                    }
                    frames
                } else {
                    vec![static_frame(reader.into_decoder()?, budget, &mut color)?]
                }
            }
            Some(image::ImageFormat::Png) => {
                let decoder = image::codecs::png::PngDecoder::with_limits(
                    BufReader::new(File::open(path)?),
                    limits(budget),
                )?;
                if decoder.is_apng()? {
                    let mut decoder = decoder;
                    let icc = decoder.icc_profile()?;
                    let mut frames =
                        animated_frames(decoder.apng()?.into_frames(), budget)?;
                    for frame in &mut frames {
                        color =
                            apply_icc(Arc::make_mut(&mut frame.pixels), icc.as_deref());
                        frame.preview = preview(&frame.pixels, PREVIEW_EDGE);
                    }
                    frames
                } else {
                    vec![static_frame(decoder, budget, &mut color)?]
                }
            }
            _ => vec![static_frame(reader.into_decoder()?, budget, &mut color)?],
        }
    };

    // Metadata is optional and must not prevent an otherwise valid image from opening
    if let Ok(file) = File::open(path)
        && let Ok(exif) =
            exif::Reader::new().read_from_container(&mut BufReader::new(file))
    {
        for tag in [
            exif::Tag::Make,
            exif::Tag::Model,
            exif::Tag::DateTimeOriginal,
            exif::Tag::ExposureTime,
            exif::Tag::FNumber,
            exif::Tag::PhotographicSensitivity,
            exif::Tag::FocalLength,
            exif::Tag::LensModel,
        ] {
            if let Some(field) = exif.get_field(tag, exif::In::PRIMARY) {
                metadata.push((
                    tag.to_string(),
                    field.display_value().with_unit(&exif).to_string(),
                ));
            }
        }
    }

    let first = frames.first().context("The image contains no frames")?;
    let size = [first.pixels.width(), first.pixels.height()];
    let thumbnail = preview(&first.pixels, 192);
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
        metadata,
    })
}

fn make_frame(pixels: RgbaImage, delay: Duration) -> DecodedFrame {
    let preview = preview(&pixels, PREVIEW_EDGE);
    DecodedFrame {
        pixels: Arc::new(pixels),
        preview,
        delay,
    }
}

fn static_frame(
    mut decoder: impl ImageDecoder,
    budget: u64,
    color: &mut String,
) -> Result<DecodedFrame> {
    decoder.set_limits(limits(budget))?;
    let (w, h) = decoder.dimensions();
    check_size(w, h, budget)?;
    let icc = decoder.icc_profile().ok().flatten();
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);

    let mut rgba = image.into_rgba8();
    *color = apply_icc(&mut rgba, icc.as_deref());
    Ok(make_frame(rgba, Duration::ZERO))
}

fn apply_icc(pixels: &mut RgbaImage, icc: Option<&[u8]>) -> String {
    let Some(icc) = icc else {
        return "sRGB / untagged".into();
    };
    let Ok(profile) = lcms2::Profile::new_icc(icc) else {
        return "Invalid ICC profile · displayed as sRGB".into();
    };
    if profile.color_space() != lcms2::ColorSpaceSignature::RgbData {
        return "Non-RGB ICC · displayed as sRGB".into();
    }
    let transform = lcms2::Transform::<u8, u8>::new_flags(
        &profile,
        lcms2::PixelFormat::RGBA_8,
        &lcms2::Profile::new_srgb(),
        lcms2::PixelFormat::RGBA_8,
        lcms2::Intent::RelativeColorimetric,
        lcms2::Flags::COPY_ALPHA,
    );
    match transform {
        Ok(transform) => {
            transform.transform_in_place(pixels.as_mut());
            "Embedded ICC → sRGB".into()
        }
        Err(_) => "ICC conversion unavailable · displayed as sRGB".into(),
    }
}

fn animated_frames(
    frames: image::Frames<'_>,
    budget: u64,
) -> Result<Vec<DecodedFrame>> {
    let mut out = Vec::new();
    let mut used = 0_u64;

    for frame in frames {
        ensure!(
            out.len() < MAX_FRAMES,
            "Animation exceeds the {MAX_FRAMES}-frame limit"
        );
        let frame = frame?;
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = Duration::from_secs_f64(
            (f64::from(num) / f64::from(den) / 1000.0).max(0.02),
        );
        let buffer = frame.into_buffer();

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
            "This animation exceeds the image memory limit. Increase it in Settings."
        );
        out.push(make_frame(buffer, delay));
    }
    ensure!(!out.is_empty(), "The animation contains no frames");
    Ok(out)
}

fn render_svg(path: &Path, edge: u32, budget: u64) -> Result<RgbaImage> {
    ensure!(
        path.metadata()?.len() <= 32 * 1024 * 1024,
        "SVG source exceeds the 32 MB limit"
    );
    let data = std::fs::read(path)?;
    ensure!(
        data.len() <= 32 * 1024 * 1024,
        "SVG source exceeds the 32 MB limit"
    );
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

pub fn thumbnail(path: &Path) -> Result<ColorImage> {
    if path.extension().is_some_and(|ext| {
        ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz")
    }) {
        return Ok(color_image(&render_svg(path, 256, 64 * 1024 * 1024)?));
    }

    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(limits(256 * 1024 * 1024));

    let mut decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    check_size(w, h, 256 * 1024 * 1024)?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let icc = decoder.icc_profile().ok().flatten();

    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let mut small = image.thumbnail(192, 128).into_rgba8();

    apply_icc(&mut small, icc.as_deref());
    Ok(color_image(&small))
}

pub fn export(
    pixels: &RgbaImage,
    path: &Path,
    rotation: u8,
    flip_x: bool,
    flip_y: bool,
) -> Result<()> {
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

    let mut image = DynamicImage::ImageRgba8(pixels.clone());

    if flip_x {
        image = image.fliph();
    }

    if flip_y {
        image = image.flipv();
    }

    image = match rotation % 4 {
        1 => image.rotate90(),
        2 => image.rotate180(),
        3 => image.rotate270(),
        _ => image,
    };

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
