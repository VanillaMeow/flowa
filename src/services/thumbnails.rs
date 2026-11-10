//! Filmstrip thumbnails for images and video posters

use std::path::PathBuf;

use egui::ColorImage;

use super::{
    image_decode::thumbnail,
    worker::{Generation, Tagged, Worker},
};
use crate::{video, viewer::formats::MediaKind};

/// The box every filmstrip thumbnail fits in, for images and video posters alike
pub const THUMBNAIL_SIZE: [u32; 2] = [192, 128];

pub struct ThumbnailRequest {
    pub path: PathBuf,
    pub kind: MediaKind,
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
    pub fn new(ctx: egui::Context, generation: &Generation) -> Self {
        let active = generation.clone();

        Self::spawn_tagged(
            "thumbnails",
            ctx,
            generation,
            move |request: ThumbnailRequest| {
                let image = if request.kind == MediaKind::Video {
                    video::thumbnail(&request.path, &active, request.generation)
                } else {
                    thumbnail(&request.path, request.budget)
                };

                Some(ThumbnailResult {
                    path: request.path,
                    generation: request.generation,
                    image: image.ok(),
                })
            },
        )
    }
}
