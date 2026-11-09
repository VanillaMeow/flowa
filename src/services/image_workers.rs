//! Bounded background workers for image tiles and thumbnails

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use egui::{ColorImage, Pos2, Rect, Vec2, pos2, vec2};
use image::RgbaImage;

use super::{
    image_decode::{color_image, thumbnail},
    resize::{self, Region},
};
use crate::viewer::{
    image_session::{DecodedFrame, DecodedImage},
    vector::VectorImage,
};

pub const TILE_SIZE: u32 = 1024;

/// The finest tile level, which only vectors reach, at 512 samples per image pixel
const FINEST_LEVEL: i8 = -9;
const COARSEST_LEVEL: i8 = 7;

/// The box every filmstrip thumbnail fits in, for images and video posters alike
pub const THUMBNAIL_SIZE: [u32; 2] = [192, 128];

const QUEUED_REQUESTS: usize = 24;
const QUEUED_RESULTS: usize = 4;

/// Requests carry the generation of the image or folder they were made for
pub trait Tagged {
    fn generation(&self) -> u64;
}

/// One background thread with bounded queues that skips work for stale generations
///
/// The owner stores the active generation; requests from older generations are
/// dropped before and after the expensive work
pub struct Worker<Request, Response> {
    pub requests: mpsc::SyncSender<Request>,
    pub results: mpsc::Receiver<Response>,
    pub generation: Arc<AtomicU64>,
}

impl<Request, Response> Worker<Request, Response>
where
    Request: Tagged + Send + 'static,
    Response: Send + 'static,
{
    fn spawn(
        ctx: egui::Context,
        mut process: impl FnMut(Request, &AtomicU64) -> Option<Response> + Send + 'static,
    ) -> Self {
        let (requests, pending) = mpsc::sync_channel::<Request>(QUEUED_REQUESTS);
        let (completed, results) = mpsc::sync_channel(QUEUED_RESULTS);
        let generation = Arc::new(AtomicU64::new(0));
        let active = Arc::clone(&generation);

        thread::spawn(move || {
            let current = |generation| active.load(Ordering::Relaxed) == generation;

            while let Ok(request) = pending.recv() {
                let generation = request.generation();

                if !current(generation) {
                    continue;
                }

                let Some(response) = process(request, &active) else {
                    continue;
                };

                // Drop work for an image or folder that was replaced meanwhile
                if !current(generation) {
                    continue;
                }

                if completed.send(response).is_err() {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub generation: u64,
    pub frame: usize,
    pub level: i8,
    pub x: u32,
    pub y: u32,
}

impl TileKey {
    /// The image region this tile paints, clipped to an image of `size`
    pub fn core(self, size: Vec2) -> Rect {
        let span = tile_span(self.level);
        let min = pos2(self.x as f32 * span, self.y as f32 * span);

        Rect::from_min_max(min, (min + Vec2::splat(span)).min(size.to_pos2()))
    }

    /// The core with a one-sample overlap, which prevents seams in bilinear filtering
    fn bounds(self, size: Vec2) -> Rect {
        let image = Rect::from_min_size(Pos2::ZERO, size);

        self.core(size)
            .expand(sample_stride(self.level))
            .intersect(image)
    }
}

/// Image pixels covered by one tile edge at `level`
pub fn tile_span(level: i8) -> f32 {
    TILE_SIZE as f32 * sample_stride(level)
}

/// The coarsest level that still supplies `density` samples per image pixel
pub fn tile_level(density: f32) -> i8 {
    let level = (1.0 / density).log2().floor();

    level.clamp(f32::from(FINEST_LEVEL), f32::from(COARSEST_LEVEL)) as i8
}

/// Image pixels per tile sample: each level doubles it, and negative levels enlarge
fn sample_stride(level: i8) -> f32 {
    2_f32.powi(i32::from(level))
}

/// What tiles are cut from: decoded pixels, or an SVG rendered at each tile's level
#[derive(Clone)]
pub enum TileSource {
    Raster(Arc<RgbaImage>),
    Vector(Arc<VectorImage>),
}

impl TileSource {
    pub fn new(image: &DecodedImage, frame: &DecodedFrame) -> Self {
        match &image.vector {
            Some(vector) => Self::Vector(Arc::clone(vector)),
            None => Self::Raster(Arc::clone(&frame.pixels)),
        }
    }

    /// Samples per image pixel beyond which tiles add no detail
    pub fn max_density(&self) -> f32 {
        match self {
            Self::Raster(_) => 1.0,
            Self::Vector(_) => 1.0 / sample_stride(FINEST_LEVEL),
        }
    }

    fn size(&self) -> Vec2 {
        let [width, height] = match self {
            Self::Raster(pixels) => [pixels.width(), pixels.height()],
            Self::Vector(vector) => vector.size(),
        };

        vec2(width as f32, height as f32)
    }

    fn build(&self, key: TileKey) -> Option<TileResult> {
        let bounds = key.bounds(self.size());

        if !bounds.is_positive() {
            return None;
        }

        let (tile, source) = match self {
            Self::Raster(pixels) => raster_tile(pixels, bounds, key.level)?,
            Self::Vector(vector) => vector_tile(vector, bounds, key.level)?,
        };

        Some(TileResult {
            key,
            image: color_image(&tile),
            source,
        })
    }
}

pub struct TileRequest {
    pub key: TileKey,
    pub source: TileSource,
}

impl Tagged for TileRequest {
    fn generation(&self) -> u64 {
        self.key.generation
    }
}

pub struct TileResult {
    pub key: TileKey,
    pub image: ColorImage,
    /// The image region the tile's texture covers
    pub source: Rect,
}

pub type TileWorker = Worker<TileRequest, TileResult>;

impl TileWorker {
    pub fn new(ctx: egui::Context) -> Self {
        Self::spawn(ctx, |request, _| request.source.build(request.key))
    }
}

/// Crops the tile's pixels, downsampling them at coarser levels
fn raster_tile(
    pixels: &RgbaImage,
    bounds: Rect,
    level: i8,
) -> Option<(RgbaImage, Rect)> {
    // Raster levels start at one sample per pixel, so the bounds are whole pixels
    let stride = 1_u32 << u32::try_from(level).ok()?;
    let region = Region {
        x: bounds.min.x as u32,
        y: bounds.min.y as u32,
        width: bounds.width() as u32,
        height: bounds.height() as u32,
    };

    let tile = if stride == 1 {
        let crop = image::imageops::crop_imm(
            pixels,
            region.x,
            region.y,
            region.width,
            region.height,
        );
        crop.to_image()
    } else {
        let size = [region.width, region.height].map(|edge| edge.div_ceil(stride));
        resize::downscale(pixels, Some(region), size).ok()?
    };

    Some((tile, bounds))
}

/// Renders the tile's region of an SVG at the level's sample density
fn vector_tile(
    vector: &VectorImage,
    bounds: Rect,
    level: i8,
) -> Option<(RgbaImage, Rect)> {
    let stride = sample_stride(level);

    // Partial samples at the image edge are rendered whole; painting clips them
    let size =
        [bounds.width(), bounds.height()].map(|edge| (edge / stride).ceil() as u32);
    let tile = vector.render(bounds.min, 1.0 / stride, size).ok()?;
    let covered = vec2(size[0] as f32, size[1] as f32) * stride;

    Some((tile, Rect::from_min_size(bounds.min, covered)))
}

pub struct ThumbnailRequest {
    pub path: PathBuf,
    pub generation: u64,
    /// The image memory limit in bytes
    pub budget: u64,
}

impl Tagged for ThumbnailRequest {
    fn generation(&self) -> u64 {
        self.generation
    }
}

pub struct ThumbnailResult {
    pub path: PathBuf,
    pub generation: u64,
    /// `None` when the file could not be decoded, so it is not retried
    pub image: Option<ColorImage>,
}

pub type ThumbnailWorker = Worker<ThumbnailRequest, ThumbnailResult>;

impl ThumbnailWorker {
    pub fn new(ctx: egui::Context) -> Self {
        Self::spawn(ctx, |request, active| {
            let image = if crate::viewer::catalog::is_video(&request.path) {
                crate::video::thumbnail(&request.path, active, request.generation)
            } else {
                thumbnail(&request.path, request.budget)
            };

            Some(ThumbnailResult {
                path: request.path,
                generation: request.generation,
                image: image.ok(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use egui::{pos2, vec2};
    use resvg::usvg::{Options, Tree};

    use super::{TileKey, TileSource, VectorImage, tile_level, tile_span};

    fn key(level: i8, x: u32, y: u32) -> TileKey {
        TileKey {
            generation: 0,
            frame: 0,
            level,
            x,
            y,
        }
    }

    #[test]
    fn tile_levels_supply_at_least_the_requested_density() {
        assert_eq!(tile_level(1.0), 0);
        assert_eq!(tile_level(0.3), 1);
        assert_eq!(tile_level(40.0), -6);
        assert_eq!(tile_level(100_000.0), -9);
        assert_eq!(tile_span(-3), 128.0);
    }

    #[test]
    fn raster_tiles_stop_at_one_sample_per_pixel() {
        let raster = TileSource::Raster(Arc::new(image::RgbaImage::new(1, 1)));

        assert_eq!(raster.max_density(), 1.0);
    }

    #[test]
    fn tile_bounds_overlap_neighbors_by_one_sample_inside_the_image() {
        let size = vec2(300.0, 100.0);

        let first = key(-3, 0, 0).bounds(size);
        assert_eq!(first.min, pos2(0.0, 0.0));
        assert_eq!(first.max, pos2(128.125, 100.0));

        let last = key(-3, 2, 0).bounds(size);
        assert_eq!(last.min, pos2(255.875, 0.0));
        assert_eq!(last.max, pos2(300.0, 100.0));
    }

    #[test]
    fn enlarged_vector_tiles_render_at_their_level() -> anyhow::Result<()> {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"/>"#;
        let tree = Tree::from_str(svg, &Options::default())?;
        let source = TileSource::Vector(Arc::new(VectorImage::new(tree, 16384.0)));

        let tile = source.build(key(-3, 0, 0)).expect("Visible tile");
        assert_eq!(tile.image.size, [160, 80]);
        assert_eq!(tile.source.max, pos2(20.0, 10.0));

        Ok(())
    }
}
