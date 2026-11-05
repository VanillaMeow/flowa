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

use super::image_decode::{color_image, thumbnail};

pub const TILE_SIZE: u32 = 1024;

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
        let (requests, requests_rx) = mpsc::sync_channel::<TileRequest>(24);
        let (results_tx, results) = mpsc::sync_channel(4);
        let generation = Arc::new(AtomicU64::new(0));
        let active_generation = Arc::clone(&generation);

        thread::spawn(move || {
            while let Ok(request) = requests_rx.recv() {
                if request.key.generation != active_generation.load(Ordering::Relaxed) {
                    continue;
                }

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
                    continue;
                }

                let region = image::imageops::crop_imm(
                    request.pixels.as_ref(),
                    x0,
                    y0,
                    x1 - x0,
                    y1 - y0,
                );
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

                // Drop work for an image that was closed while the tile was built
                if request.key.generation != active_generation.load(Ordering::Relaxed) {
                    continue;
                }

                if results_tx.send(result).is_err() {
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
        let (requests, requests_rx) = mpsc::sync_channel::<ThumbnailRequest>(24);
        let (results_tx, results) = mpsc::sync_channel(4);
        let generation = Arc::new(AtomicU64::new(0));
        let active_generation = Arc::clone(&generation);

        thread::spawn(move || {
            while let Ok(request) = requests_rx.recv() {
                if active_generation.load(Ordering::Relaxed) != request.generation {
                    continue;
                }

                let result = ThumbnailResult {
                    path: request.path,
                    generation: request.generation,
                    image: None,
                };
                let image = if crate::viewer::catalog::is_video(&result.path) {
                    crate::video::thumbnail(
                        &result.path,
                        &active_generation,
                        result.generation,
                    )
                } else {
                    thumbnail(&result.path)
                }
                .ok();

                if active_generation.load(Ordering::Relaxed) != request.generation {
                    continue;
                }

                if results_tx
                    .send(ThumbnailResult { image, ..result })
                    .is_err()
                {
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
