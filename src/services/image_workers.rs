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

use egui::ColorImage;
use image::RgbaImage;

use super::{
    image_decode::{color_image, thumbnail},
    resize::{self, Region},
};

pub const TILE_SIZE: u32 = 1024;

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
    pub level: u8,
    pub x: u32,
    pub y: u32,
}

pub struct TileRequest {
    pub key: TileKey,
    pub pixels: Arc<RgbaImage>,
}

impl Tagged for TileRequest {
    fn generation(&self) -> u64 {
        self.key.generation
    }
}

pub struct TileResult {
    pub key: TileKey,
    pub image: ColorImage,
    pub source: egui::Rect,
}

pub type TileWorker = Worker<TileRequest, TileResult>;

impl TileWorker {
    pub fn new(ctx: egui::Context) -> Self {
        Self::spawn(ctx, |request, _| build_tile(&request))
    }
}

/// Crops and downsamples one tile, with overlap for seamless bilinear filtering
fn build_tile(request: &TileRequest) -> Option<TileResult> {
    let size = request.pixels.dimensions();

    // Each level halves the resolution while covering more source pixels
    let stride = 1_u32 << request.key.level;
    let span = TILE_SIZE * stride;

    // A one-sample overlap prevents seams during bilinear filtering
    let x0 = (request.key.x * span).saturating_sub(stride);
    let y0 = (request.key.y * span).saturating_sub(stride);
    let x1 = ((request.key.x + 1) * span + stride).min(size.0);
    let y1 = ((request.key.y + 1) * span + stride).min(size.1);

    if x0 >= x1 || y0 >= y1 {
        return None;
    }

    let region = Region {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    };
    let tile = if stride == 1 {
        image::imageops::crop_imm(
            request.pixels.as_ref(),
            x0,
            y0,
            region.width,
            region.height,
        )
        .to_image()
    } else {
        let width = region.width.div_ceil(stride);
        let height = region.height.div_ceil(stride);
        resize::downscale(&request.pixels, Some(region), width, height).ok()?
    };

    Some(TileResult {
        key: request.key,
        image: color_image(&tile),
        source: egui::Rect::from_min_max(
            egui::pos2(x0 as f32, y0 as f32),
            egui::pos2(x1 as f32, y1 as f32),
        ),
    })
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
