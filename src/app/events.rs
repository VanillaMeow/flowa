//! Apply background results without blocking the event loop

use super::*;

const TILE_UPLOADS_PER_FRAME: usize = 4;
const THUMBNAIL_UPLOADS_PER_FRAME: usize = 4;
const MAX_CACHED_TILES: usize = 46;
const MAX_CACHED_THUMBNAILS: usize = 128;

impl Flowa {
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        self.poll_catalog(ctx);
        self.poll_images(ctx);
        self.poll_tiles(ctx);
        self.poll_thumbnails(ctx);
        self.poll_actions(ctx);
    }

    fn poll_catalog(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.catalog_rx.try_recv() {
            // Folder scans can finish after a newer folder has already opened
            if result.serial != self.scan_serial {
                continue;
            }
            match result.result {
                Ok(mut files) => {
                    catalog::sort(
                        &mut files,
                        self.settings.sort,
                        self.settings.reverse_sort,
                    );
                    let index = result
                        .selected
                        .and_then(|p| files.iter().position(|f| f.path == p))
                        .unwrap_or(0);
                    self.files = files;
                    self.folder = Some(result.folder);
                    self.thumbnails.clear();
                    self.thumb_order.clear();
                    self.thumb_pending.clear();
                    self.thumbnail_worker
                        .generation
                        .store(self.scan_serial, Ordering::Relaxed);
                    if self.files.is_empty() {
                        self.index = 0;
                        self.clear_image();
                        self.load_error = Some(
                            "There are no supported images in this folder.".into(),
                        );
                    } else {
                        self.select(index, ctx);
                    }
                }
                Err(error) => {
                    self.loading = false;
                    self.notify(format!("{error:#}"), true);
                }
            }
        }
    }

    fn poll_images(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.loader.results.try_recv() {
            if result.generation != self.generation {
                continue;
            }
            self.loading = false;

            match result.result {
                Ok(decoded) => {
                    let first = &decoded.frames[0];
                    self.preview_texture = Some(ctx.load_texture(
                        "image-preview",
                        first.preview.clone(),
                        self.texture_options(),
                    ));
                    self.preview_frame = 0;
                    self.frame_index = 0;
                    self.playing = true;
                    self.frame_deadline = Instant::now() + first.delay;
                    if let Some(path) = self.current().map(|e| e.path.clone()) {
                        let thumbnail = ctx.load_texture(
                            format!("thumbnail:{}", path.display()),
                            decoded.thumbnail.clone(),
                            TextureOptions::LINEAR,
                        );
                        self.cache_thumbnail(path, Some(thumbnail));
                    }
                    self.decoded = Some(decoded);
                    if let Some(percent) = self.launch.zoom.take() {
                        self.view.fit = false;
                        self.view.zoom = percent / 100.0 / ctx.pixels_per_point();
                    }
                    self.next_slide = Instant::now()
                        + Duration::from_secs_f32(self.settings.slideshow_seconds);
                }
                Err(error) => {
                    self.load_error = Some(format!("{error:#}"));
                    self.decoded = None;
                }
            }
        }
    }

    fn poll_tiles(&mut self, ctx: &egui::Context) {
        // Bound texture work per frame so decoding a large image does not stall input
        for _ in 0..TILE_UPLOADS_PER_FRAME {
            let Ok(result) = self.tile_worker.results.try_recv() else {
                break;
            };

            self.pending_tiles.remove(&result.key);
            if result.key.generation != self.generation {
                continue;
            }

            let texture = ctx.load_texture(
                format!("tile:{:?}", result.key),
                result.image,
                self.texture_options(),
            );
            self.tiles.insert(
                result.key,
                TileTexture {
                    texture,
                    source: result.source,
                    touched: self.paint_counter,
                },
            );

            // Evict the least recently painted tile before the cache exceeds its budget
            while self.tiles.len() > MAX_CACHED_TILES {
                if let Some(key) = self
                    .tiles
                    .iter()
                    .min_by_key(|(_, v)| v.touched)
                    .map(|(k, _)| *k)
                {
                    self.tiles.remove(&key);
                } else {
                    break;
                }
            }
            ctx.request_repaint();
        }
    }

    fn poll_thumbnails(&mut self, ctx: &egui::Context) {
        for _ in 0..THUMBNAIL_UPLOADS_PER_FRAME {
            let Ok(result) = self.thumbnail_worker.results.try_recv() else {
                break;
            };
            self.thumb_pending.remove(&result.path);
            if result.generation != self.scan_serial {
                continue;
            }
            let texture = result.image.map(|image| {
                ctx.load_texture(
                    format!("thumb:{}", result.path.display()),
                    image,
                    TextureOptions::LINEAR,
                )
            });
            self.cache_thumbnail(result.path, texture);
        }
    }

    fn poll_actions(&mut self, ctx: &egui::Context) {
        while let Ok(outcome) = self.actions.results.try_recv() {
            self.busy = None;
            match outcome {
                Ok(Outcome::Picked(Some(path))) => self.open_path(path, ctx),
                Ok(Outcome::Picked(None) | Outcome::Revealed) => {}
                Ok(Outcome::Copied) => self.notify("Image copied", false),
                Ok(Outcome::Exported) => self.notify("Copy exported", false),
                Ok(Outcome::Renamed { old, new }) => {
                    self.settings.recent.retain(|p| *p != old);
                    if let Some(entry) = self.files.iter_mut().find(|f| f.path == old)
                        && let Ok(new_entry) = Entry::from_path(new.clone())
                    {
                        *entry = new_entry;
                    }
                    self.thumbnails.remove(&old);
                    self.sort();
                    self.settings.remember(new);
                    if let Some(entry) = self.current() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                            "{} — flowa",
                            entry.name
                        )));
                    }
                    self.notify("Image renamed", false);
                    if self.loading {
                        self.select(self.index, ctx);
                    }
                }
                Ok(Outcome::Trashed(path)) => {
                    self.settings.recent.retain(|p| *p != path);
                    self.thumbnails.remove(&path);
                    if let Some(i) = self.files.iter().position(|f| f.path == path) {
                        self.files.remove(i);
                        if self.files.is_empty() {
                            self.clear_image();
                            self.index = 0;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                                "flowa".into(),
                            ));
                        } else {
                            self.select(i.min(self.files.len() - 1), ctx);
                        }
                    }
                    self.notify("Moved to the system trash", false);
                }
                Err(error) => self.notify(format!("{error:#}"), true),
            }
        }
    }

    pub(super) fn cache_thumbnail(
        &mut self,
        path: PathBuf,
        texture: Option<TextureHandle>,
    ) {
        self.thumb_order.retain(|p| *p != path);
        self.thumb_order.push(path.clone());
        // A cached None also prevents retrying a corrupt thumbnail every frame
        self.thumbnails.insert(path, texture);

        while self.thumb_order.len() > MAX_CACHED_THUMBNAILS {
            let old = self.thumb_order.remove(0);
            self.thumbnails.remove(&old);
        }
    }
}
