//! Bounded background workers for decoding, tiles, and thumbnails

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Context, Result};
use egui::ColorImage;
use image::RgbaImage;

use crate::viewer::image_session::DecodedImage;

use super::image_decode::{color_image, decode, thumbnail};

pub const TILE_SIZE: u32 = 1024;

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
        let (sender, requests_rx) = mpsc::channel::<LoadRequest>();
        let (results_tx, results) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let active_generation = Arc::clone(&generation);

        thread::spawn(move || {
            while let Ok(mut request) = requests_rx.recv() {
                // Skip intermediate selections when the user holds an arrow key
                while let Ok(new_request) = requests_rx.try_recv() {
                    request = new_request;
                }

                if active_generation.load(Ordering::Relaxed) != request.generation {
                    continue;
                }

                let result = decode(&request.path, request.budget).with_context(|| {
                    format!("Could not open {}", request.path.display())
                });

                if active_generation.load(Ordering::Relaxed) != request.generation {
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
                let image = thumbnail(&result.path).ok();

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
