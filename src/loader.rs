//! CPU decoding runs outside the event loop. Only visible full-resolution tiles reach the GPU.
use anyhow::{Context, Result, bail, ensure};
use egui::ColorImage;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageReader, RgbaImage};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub const TILE_SIZE: u32 = 1024;
const PREVIEW_EDGE: u32 = 2048;
const MAX_FRAMES: usize = 2000;

pub struct DecodedFrame {
    pub pixels: Arc<RgbaImage>,
    pub preview: ColorImage,
    pub delay: Duration,
}
pub struct DecodedImage {
    pub frames: Vec<DecodedFrame>,
    pub thumbnail: ColorImage,
    pub size: [u32; 2],
    pub format: String,
    pub file_size: u64,
    pub color: String,
    pub metadata: Vec<(String, String)>,
}
impl DecodedImage {
    pub fn duration(&self) -> Duration {
        self.frames.iter().map(|f| f.delay).sum()
    }
}

fn limits(budget: u64) -> image::Limits {
    let mut l = image::Limits::default();
    l.max_alloc = Some(budget);
    l.max_image_width = Some(131_072);
    l.max_image_height = Some(131_072);
    l
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
    let width = (u64::from(pixels.width()) * u64::from(edge) / u64::from(longest)).max(1) as u32;
    let height = (u64::from(pixels.height()) * u64::from(edge) / u64::from(longest)).max(1) as u32;
    let resized = image::imageops::thumbnail(pixels, width, height);
    color_image(&resized)
}
pub fn color_image(pixels: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        pixels.as_raw(),
    )
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
                let mut decoder =
                    image::codecs::gif::GifDecoder::new(BufReader::new(File::open(path)?))?;
                decoder.set_limits(limits(budget))?;
                check_size(decoder.dimensions().0, decoder.dimensions().1, budget)?;
                animated_frames(decoder.into_frames(), budget)?
            }
            Some(image::ImageFormat::WebP) => {
                let mut decoder =
                    image::codecs::webp::WebPDecoder::new(BufReader::new(File::open(path)?))?;
                decoder.set_limits(limits(budget))?;
                if decoder.has_animation() {
                    let icc = decoder.icc_profile()?;
                    let mut frames = animated_frames(decoder.into_frames(), budget)?;
                    for frame in &mut frames {
                        color = apply_icc(Arc::make_mut(&mut frame.pixels), icc.as_deref());
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
                    let mut frames = animated_frames(decoder.apng()?.into_frames(), budget)?;
                    for frame in &mut frames {
                        color = apply_icc(Arc::make_mut(&mut frame.pixels), icc.as_deref());
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
    if let Ok(file) = File::open(path)
        && let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file))
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
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    let mut rgba = img.into_rgba8();
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
fn animated_frames(frames: image::Frames<'_>, budget: u64) -> Result<Vec<DecodedFrame>> {
    let mut out = Vec::new();
    let mut used = 0_u64;
    for frame in frames {
        ensure!(
            out.len() < MAX_FRAMES,
            "Animation exceeds the {MAX_FRAMES}-frame limit"
        );
        let frame = frame?;
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = Duration::from_secs_f64((f64::from(num) / f64::from(den) / 1000.0).max(0.02));
        let buffer = frame.into_buffer();
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
    // Do not resolve external files from an SVG. Embedded data images remain supported.
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
    // tiny-skia stores premultiplied colors; the viewer uses straight-alpha image data.
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
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz"))
    {
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
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    let mut small = img.thumbnail(192, 128).into_rgba8();
    apply_icc(&mut small, icc.as_deref());
    Ok(color_image(&small))
}

pub struct LoadRequest {
    pub generation: u64,
    pub path: PathBuf,
    pub budget: u64,
}
pub struct LoadResult {
    pub generation: u64,
    pub result: Result<DecodedImage>,
}
pub struct Loader {
    sender: mpsc::Sender<LoadRequest>,
    pub results: mpsc::Receiver<LoadResult>,
    generation: Arc<AtomicU64>,
}
impl Loader {
    pub fn new(ctx: egui::Context) -> Self {
        let (sender, requests) = mpsc::channel::<LoadRequest>();
        let (results_tx, results) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        thread::spawn(move || {
            while let Ok(mut request) = requests.recv() {
                while let Ok(new) = requests.try_recv() {
                    request = new;
                }
                if current.load(Ordering::Relaxed) != request.generation {
                    continue;
                }
                let result = decode(&request.path, request.budget)
                    .with_context(|| format!("Could not open {}", request.path.display()));
                if current.load(Ordering::Relaxed) != request.generation {
                    continue;
                }
                if results_tx
                    .send(LoadResult {
                        generation: request.generation,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            sender,
            results,
            generation,
        }
    }
    pub fn request(&self, request: LoadRequest) {
        self.generation.store(request.generation, Ordering::Relaxed);
        let _ = self.sender.send(request);
    }
    pub fn cancel(&self, generation: u64) {
        self.generation.store(generation, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub generation: u64,
    pub frame: usize,
    pub level: u8,
    pub x: u32,
    pub y: u32,
}
pub struct TileRequest {
    pub key: TileKey,
    pub pixels: Arc<RgbaImage>,
}
pub struct TileResult {
    pub key: TileKey,
    pub image: ColorImage,
    pub source: egui::Rect,
}
pub struct TileWorker {
    pub requests: mpsc::SyncSender<TileRequest>,
    pub results: mpsc::Receiver<TileResult>,
    pub generation: Arc<AtomicU64>,
}
impl TileWorker {
    pub fn new(ctx: egui::Context) -> Self {
        let (requests, rx) = mpsc::sync_channel::<TileRequest>(24);
        let (tx, results) = mpsc::sync_channel(4);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                if request.key.generation != current.load(Ordering::Relaxed) {
                    continue;
                }
                let size = request.pixels.dimensions();
                let stride = 1_u32 << request.key.level;
                let span = TILE_SIZE * stride;
                let x0 = (request.key.x * span).saturating_sub(stride);
                let y0 = (request.key.y * span).saturating_sub(stride);
                let x1 = ((request.key.x + 1) * span + stride).min(size.0);
                let y1 = ((request.key.y + 1) * span + stride).min(size.1);
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }
                let region =
                    image::imageops::crop_imm(request.pixels.as_ref(), x0, y0, x1 - x0, y1 - y0);
                let tile = if stride == 1 {
                    region.to_image()
                } else {
                    image::imageops::resize(
                        &*region,
                        (x1 - x0).div_ceil(stride),
                        (y1 - y0).div_ceil(stride),
                        image::imageops::FilterType::Triangle,
                    )
                };
                let result = TileResult {
                    key: request.key,
                    image: color_image(&tile),
                    source: egui::Rect::from_min_max(
                        egui::pos2(x0 as f32, y0 as f32),
                        egui::pos2(x1 as f32, y1 as f32),
                    ),
                };
                if request.key.generation != current.load(Ordering::Relaxed) {
                    continue;
                }
                if tx.send(result).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            requests,
            results,
            generation,
        }
    }
}

pub struct ThumbnailRequest {
    pub path: PathBuf,
    pub generation: u64,
}
pub struct ThumbnailResult {
    pub path: PathBuf,
    pub generation: u64,
    pub image: Option<ColorImage>,
}
pub struct ThumbnailWorker {
    pub requests: mpsc::SyncSender<ThumbnailRequest>,
    pub results: mpsc::Receiver<ThumbnailResult>,
    pub generation: Arc<AtomicU64>,
}
impl ThumbnailWorker {
    pub fn new(ctx: egui::Context) -> Self {
        let (requests, rx) = mpsc::sync_channel::<ThumbnailRequest>(24);
        let (tx, results) = mpsc::sync_channel(4);
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                if current.load(Ordering::Relaxed) != request.generation {
                    continue;
                }
                let result = ThumbnailResult {
                    path: request.path,
                    generation: request.generation,
                    image: None,
                };
                let image = thumbnail(&result.path).ok();
                if current.load(Ordering::Relaxed) != request.generation {
                    continue;
                }
                if tx.send(ThumbnailResult { image, ..result }).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            requests,
            results,
            generation,
        }
    }
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
    // Encode to a temporary sibling and replace only once encoding succeeds.
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_decode_and_export_preserve_alpha_and_apply_rotation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("source.png");
        let output = dir.path().join("export.png");
        let pixels = RgbaImage::from_fn(30, 20, |x, y| image::Rgba([x as u8, y as u8, 90, 128]));
        pixels.save(&input)?;
        let loaded = decode(&input, 32 * 1024 * 1024)?;
        assert_eq!(loaded.size, [30, 20]);
        export(&loaded.frames[0].pixels, &output, 1, false, false)?;
        let result = image::open(output)?.into_rgba8();
        assert_eq!(result.dimensions(), (20, 30));
        assert_eq!(result.get_pixel(0, 0)[3], 128);
        Ok(())
    }
    #[test]
    fn corrupt_images_fail_without_panicking() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("bad.png");
        std::fs::write(&path, b"not an image")?;
        assert!(decode(&path, 32 * 1024 * 1024).is_err());
        Ok(())
    }
    #[test]
    fn oversized_image_headers_respect_the_memory_budget() {
        assert!(check_size(50_000, 50_000, 64 * 1024 * 1024).is_err());
    }
    #[test]
    fn previews_preserve_aspect_ratio_and_never_enlarge_small_images() {
        let small = RgbaImage::new(30, 20);
        assert_eq!(preview(&small, 2048).size, [30, 20]);
        let wide = RgbaImage::new(640, 320);
        assert_eq!(preview(&wide, 192).size, [192, 96]);
    }
    #[test]
    fn svg_alpha_is_unpremultiplied() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("transparent.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000" opacity="0.5"/></svg>"##,
        )?;
        let decoded = decode(&path, 32 * 1024 * 1024)?;
        let px = decoded.frames[0].pixels.get_pixel(5, 5);
        assert_eq!(px[0], 255);
        assert!((126..=129).contains(&px[3]));
        Ok(())
    }
    #[test]
    fn gif_animation_retains_frame_delays() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("animated.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path)?);
        for c in [40, 200] {
            encoder.encode_frame(image::Frame::from_parts(
                RgbaImage::from_pixel(8, 8, image::Rgba([c, 0, 0, 255])),
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            ))?;
        }
        drop(encoder);
        let decoded = decode(&path, 1024 * 1024)?;
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.frames[0].delay, Duration::from_millis(100));
        Ok(())
    }
}
