//! Preview textures and bounded uploads for visible image tiles

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use egui::{Color32, Pos2, Rect, TextureHandle, TextureOptions, Vec2, vec2};

use crate::{
    services::{
        tiles::{self, TileKey, TileRequest, TileSource, TileWorker},
        worker::Generation,
    },
    settings::{Canvas, Settings},
    ui::theme::{self, Colors},
    viewer::{image_session::ImageSession, transform::View},
};

use super::geometry;

const TILE_UPLOADS_PER_FRAME: usize = 4;
const TILE_REQUESTS_PER_FRAME: usize = 4;
/// About 184 MiB of 1024² RGBA textures, enough to cover a 4K canvas with headroom
const MAX_CACHED_TILES: usize = 46;

struct TileTexture {
    texture: TextureHandle,
    source: Rect,
    touched: u64,
}

pub(crate) struct FrozenPreview {
    texture: TextureHandle,
    size: Vec2,
    view: View,
}

impl FrozenPreview {
    pub fn paint(
        &self,
        painter: &egui::Painter,
        canvas: Rect,
        settings: &Settings,
        colors: Colors,
    ) {
        let image_rect = self.view.image_rect(self.size, canvas);
        let source = Rect::from_min_size(Pos2::ZERO, self.size);

        paint_backdrop(
            painter,
            image_rect,
            canvas_color(settings, colors),
            settings,
        );
        paint_source(
            painter,
            self.texture.id(),
            source,
            self.size,
            canvas,
            &self.view,
        );
    }
}

pub(crate) struct RenderFrame<'a> {
    pub image: &'a ImageSession,
    pub view: &'a View,
    pub canvas: Rect,
    pub settings: &'a Settings,
    pub colors: Colors,
}

pub(crate) struct ImageRenderer {
    preview: Option<TextureHandle>,
    preview_frame: usize,
    worker: TileWorker,
    generation: Generation,
    tiles: HashMap<TileKey, TileTexture>,
    pending_tiles: HashSet<TileKey>,
    paint_counter: u64,
    options: TextureOptions,
}

impl ImageRenderer {
    pub fn new(ctx: egui::Context, nearest: bool) -> Self {
        let generation = Generation::default();

        Self {
            preview: None,
            preview_frame: usize::MAX,
            worker: TileWorker::new(ctx, &generation),
            generation,
            tiles: HashMap::new(),
            pending_tiles: HashSet::new(),
            paint_counter: 0,
            options: texture_options(nearest),
        }
    }

    pub fn reset(&mut self, generation: u64) {
        self.generation.set(generation);
        self.preview = None;
        self.preview_frame = usize::MAX;
        self.tiles.clear();
        self.pending_tiles.clear();
    }

    pub fn freeze(&mut self, size: Vec2, view: &View) -> Option<FrozenPreview> {
        Some(FrozenPreview {
            texture: self.preview.take()?,
            size,
            view: view.clone(),
        })
    }

    pub fn sync_frame(&mut self, ctx: &egui::Context, image: &ImageSession) {
        if self.preview_frame == image.playback.frame_index {
            return;
        }

        let Some(decoded) = &image.decoded else {
            return;
        };
        let Some(frame) = decoded.frames.get(image.playback.frame_index) else {
            return;
        };

        if let Some(texture) = &mut self.preview {
            texture.set(frame.preview.clone(), self.options);
        } else {
            self.preview = Some(ctx.load_texture(
                "image-preview",
                frame.preview.clone(),
                self.options,
            ));
        }

        self.preview_frame = image.playback.frame_index;
    }

    pub fn set_sampling(
        &mut self,
        ctx: &egui::Context,
        image: &ImageSession,
        nearest: bool,
    ) {
        self.options = texture_options(nearest);
        self.tiles.clear();
        self.preview_frame = usize::MAX;

        self.sync_frame(ctx, image);
    }

    pub fn begin_pass(&mut self) {
        self.paint_counter = self.paint_counter.wrapping_add(1);
    }

    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        ctx: &egui::Context,
        frame: RenderFrame<'_>,
    ) {
        let Some(decoded) = &frame.image.decoded else {
            return;
        };

        let size = vec2(decoded.size[0] as f32, decoded.size[1] as f32);
        let base = canvas_color(frame.settings, frame.colors);
        let image_rect = frame.view.image_rect(size, frame.canvas);

        paint_backdrop(painter, image_rect, base, frame.settings);
        if let Some(texture) = &self.preview {
            paint_source(
                painter,
                texture.id(),
                Rect::from_min_size(Pos2::ZERO, size),
                size,
                frame.canvas,
                frame.view,
            );
        }

        self.paint_tiles(painter, ctx, &frame, size, base);
    }

    pub fn poll(&mut self, ctx: &egui::Context, generation: u64) {
        // Bound texture work per frame so decoding a large image does not stall input
        for _ in 0..TILE_UPLOADS_PER_FRAME {
            let Ok(result) = self.worker.results.try_recv() else {
                break;
            };

            self.pending_tiles.remove(&result.key);
            if result.key.generation != generation {
                continue;
            }

            let texture = ctx.load_texture(
                format!("tile:{:?}", result.key),
                result.image,
                self.options,
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

    fn paint_tiles(
        &mut self,
        painter: &egui::Painter,
        ctx: &egui::Context,
        frame: &RenderFrame<'_>,
        size: Vec2,
        base: Color32,
    ) {
        let canvas = frame.canvas;
        let Some(decoded) = frame.image.decoded.as_ref() else {
            return;
        };

        if decoded.frames.len() > 1 && frame.image.playback.playing {
            return;
        }

        let Some(decoded_frame) = decoded.frames.get(frame.image.playback.frame_index)
        else {
            return;
        };

        // Tiles are needed only where the screen samples more finely than the preview
        let source = TileSource::new(decoded, decoded_frame);
        let density = frame
            .view
            .pixel_scale(ctx.pixels_per_point())
            .min(source.max_density());
        let preview_density = decoded_frame.preview.size[0] as f32 / size.x;

        if density <= preview_density * 1.02 {
            return;
        }

        let level = tiles::tile_level(density);
        let span = tiles::tile_span(level);
        let visible = frame.view.visible_source(size, canvas);

        if !visible.is_positive() {
            return;
        }

        let keys = visible_tile_keys(frame, size, level, span, visible);
        let mut requests = 0;

        for key in keys {
            if let Some(tile) = self.tiles.get_mut(&key) {
                tile.touched = self.paint_counter;
                paint_resident_tile(painter, tile, key, frame, size, base);
            } else if requests < TILE_REQUESTS_PER_FRAME
                && self.request_tile(key, &source)
            {
                requests += 1;
            }
        }

        if requests > 0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn request_tile(&mut self, key: TileKey, source: &TileSource) -> bool {
        if self.pending_tiles.contains(&key) {
            return false;
        }

        let request = TileRequest {
            key,
            source: source.clone(),
        };

        if !self.worker.request(request) {
            return false;
        }

        self.pending_tiles.insert(key);
        true
    }
}

fn visible_tile_keys(
    frame: &RenderFrame<'_>,
    size: Vec2,
    level: i8,
    span: f32,
    visible: Rect,
) -> Vec<TileKey> {
    let first = (visible.min.max(Pos2::ZERO) / span).floor();
    let end = (visible.max.min(size.to_pos2()) / span).ceil();
    let [x0, y0, x1, y1] = [first.x, first.y, end.x, end.y].map(|edge| edge as u32);

    let mut keys = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            keys.push(TileKey {
                generation: frame.image.generation(),
                frame: frame.image.playback.frame_index,
                level,
                x,
                y,
            });
        }
    }

    // Resolve the center first so detail appears where the user is looking
    keys.sort_by_key(|key| {
        let dx = (key.x as f32 + 0.5) * span - visible.center().x;
        let dy = (key.y as f32 + 0.5) * span - visible.center().y;
        (dx * dx + dy * dy) as u64
    });

    keys
}

fn paint_resident_tile(
    painter: &egui::Painter,
    tile: &TileTexture,
    key: TileKey,
    frame: &RenderFrame<'_>,
    size: Vec2,
    base: Color32,
) {
    let canvas = frame.canvas;
    let image_rect = frame.view.image_rect(size, canvas);
    let texture_id = tile.texture.id();
    let source = tile.source;
    let core = key.core(size);
    let core_points = [
        core.left_top(),
        core.right_top(),
        core.right_bottom(),
        core.left_bottom(),
    ]
    .map(|p| frame.view.image_to_screen(p.to_vec2(), size, canvas));
    let clip = Rect::from_points(&core_points).intersect(canvas);
    let painter = painter.with_clip_rect(clip);

    // Clear the preview here so translucent pixels are not blended twice
    painter.rect_filled(clip, 0., base);
    paint_backdrop(&painter, image_rect, base, frame.settings);
    paint_source(&painter, texture_id, source, size, canvas, frame.view);
}

fn texture_options(nearest: bool) -> TextureOptions {
    if nearest {
        TextureOptions::NEAREST
    } else {
        TextureOptions::LINEAR
    }
}

pub(crate) fn canvas_color(settings: &Settings, colors: Colors) -> Color32 {
    match settings.canvas {
        Canvas::Theme => colors.canvas,
        Canvas::Dark => Color32::from_rgb(24, 24, 26),
        Canvas::Black => Color32::BLACK,
        Canvas::White => Color32::WHITE,
        Canvas::Custom => Color32::from_rgb(
            settings.canvas_color[0],
            settings.canvas_color[1],
            settings.canvas_color[2],
        ),
    }
}

fn paint_backdrop(
    painter: &egui::Painter,
    image_rect: Rect,
    base: Color32,
    settings: &Settings,
) {
    if !settings.checkerboard {
        return;
    }

    let visible = image_rect.intersect(painter.clip_rect());

    if !visible.is_positive() {
        return;
    }

    let square = settings.checker_size;
    let contrast = if theme::is_dark(base) {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    let alternate: Color32 = egui::lerp(
        egui::Rgba::from(base)..=egui::Rgba::from(contrast),
        settings.checker_contrast,
    )
    .into();

    let x0 = ((visible.left() - image_rect.left()) / square).floor() as i32;
    let y0 = ((visible.top() - image_rect.top()) / square).floor() as i32;
    let x1 = ((visible.right() - image_rect.left()) / square).ceil() as i32;
    let y1 = ((visible.bottom() - image_rect.top()) / square).ceil() as i32;

    let painter = painter.with_clip_rect(visible);

    for y in y0..y1 {
        for x in x0..x1 {
            if (x + y) % 2 == 0 {
                painter.rect_filled(
                    Rect::from_min_size(
                        image_rect.min + vec2(x as f32 * square, y as f32 * square),
                        Vec2::splat(square),
                    ),
                    0.,
                    alternate,
                );
            }
        }
    }
}

pub(crate) fn paint_source(
    painter: &egui::Painter,
    texture: egui::TextureId,
    source: Rect,
    size: Vec2,
    canvas: Rect,
    view: &View,
) {
    let points = [
        source.left_top(),
        source.right_top(),
        source.right_bottom(),
        source.left_bottom(),
    ]
    .map(|point| view.image_to_screen(point.to_vec2(), size, canvas));

    geometry::uv_quad(painter, texture, points);
}
