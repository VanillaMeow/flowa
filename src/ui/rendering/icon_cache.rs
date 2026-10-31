//! Rasterized SVG icons cached by glyph and physical pixel size

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use egui::{
    Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, Vec2,
    emath::GuiRounding,
};

use crate::ui::icons::Icon;

const MAX_CACHED_TEXTURES: usize = 64;
const UNUSED_FRAME_LIMIT: u64 = 120;

type CachedTexture = Result<TextureHandle, Arc<str>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    icon: Icon,
    pixels: u32,
}

struct CacheEntry {
    texture: CachedTexture,
    last_used: u64,
}

#[derive(Default)]
struct IconCache {
    entries: HashMap<CacheKey, CacheEntry>,
    frame: u64,
}

impl IconCache {
    fn get(&mut self, key: CacheKey) -> Option<CachedTexture> {
        let entry = self.entries.get_mut(&key)?;
        entry.last_used = self.frame;

        Some(entry.texture.clone())
    }

    fn insert(&mut self, key: CacheKey, texture: CachedTexture) {
        if self.entries.len() >= MAX_CACHED_TEXTURES {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key);

            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }

        self.entries.insert(
            key,
            CacheEntry {
                texture,
                last_used: self.frame,
            },
        );
    }
}

impl egui::cache::CacheTrait for IconCache {
    fn update(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;

        self.entries.retain(|_, entry| {
            frame.wrapping_sub(entry.last_used) <= UNUSED_FRAME_LIMIT
        });
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

pub(crate) fn paint(
    painter: &egui::Painter,
    icon: Icon,
    center: Pos2,
    size: f32,
    color: Color32,
) {
    let pixels_per_point = painter.pixels_per_point();
    let pixels = (size * pixels_per_point).round().max(1.) as u32;
    let key = CacheKey { icon, pixels };

    let Ok(texture) = texture(painter.ctx(), key) else {
        return;
    };

    // Align the bitmap to device pixels so thin strokes stay crisp
    let logical_size = pixels as f32 / pixels_per_point;
    let min =
        (center - Vec2::splat(logical_size / 2.)).round_to_pixels(pixels_per_point);
    let rect = Rect::from_min_size(min, Vec2::splat(logical_size));
    let uv = Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.));

    painter.image(texture.id(), rect, uv, color);
}

fn texture(ctx: &egui::Context, key: CacheKey) -> CachedTexture {
    let cached = ctx.memory_mut(|memory| memory.caches.cache::<IconCache>().get(key));

    if let Some(cached) = cached {
        return cached;
    }

    // Rasterization and GPU uploads run after releasing egui's memory lock
    let rendered = rasterize(key.icon, key.pixels)
        .map(|image| {
            ctx.load_texture(
                format!("lucide:{:?}:{}px", key.icon, key.pixels),
                image,
                TextureOptions::LINEAR,
            )
        })
        .map_err(|error| Arc::<str>::from(format!("{error:#}")));

    if let Err(error) = &rendered {
        log::error!("Could not render Lucide icon {:?}: {error}", key.icon);
    }

    // Failed assets are cached too, avoiding repeated parsing and error logs
    ctx.memory_mut(|memory| {
        memory
            .caches
            .cache::<IconCache>()
            .insert(key, rendered.clone());
    });

    rendered
}

fn rasterize(icon: Icon, pixels: u32) -> Result<ColorImage> {
    let svg = icon.svg().replace("currentColor", "#ffffff");
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
        .context("Invalid embedded Lucide SVG")?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixels, pixels)
        .context("Could not allocate icon pixels")?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        pixels as f32 / tree.size().width(),
        pixels as f32 / tree.size().height(),
    );

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Tiny-skia and egui both use premultiplied alpha for these white masks
    Ok(ColorImage::from_rgba_premultiplied(
        [pixels as usize, pixels as usize],
        pixmap.data(),
    ))
}

#[cfg(test)]
mod tests {
    use egui::cache::CacheTrait;

    use super::*;

    const ICONS: [Icon; 18] = [
        Icon::Open,
        Icon::Left,
        Icon::Right,
        Icon::Fit,
        Icon::Actual,
        Icon::Minus,
        Icon::Plus,
        Icon::Rotate,
        Icon::Filmstrip,
        Icon::Info,
        Icon::Settings,
        Icon::Fullscreen,
        Icon::Close,
        Icon::Play,
        Icon::Pause,
        Icon::Image,
        Icon::Check,
        Icon::Alert,
    ];

    #[test]
    fn embedded_icons_render_as_white_alpha_masks_at_ui_sizes() -> Result<()> {
        for icon in ICONS {
            for pixels in [18, 27, 36, 44] {
                let image = rasterize(icon, pixels)?;

                assert_eq!(image.size, [pixels as usize; 2]);
                assert!(
                    image.pixels.iter().any(|pixel| pixel.a() != 0),
                    "Empty icon: {icon:?}"
                );
                assert!(
                    image.pixels.iter().any(|pixel| pixel.a() == 0),
                    "Opaque background: {icon:?}"
                );
                assert!(
                    image.pixels.iter().all(|pixel| {
                        pixel.r() == pixel.a()
                            && pixel.g() == pixel.a()
                            && pixel.b() == pixel.a()
                    }),
                    "Icon cannot be tinted correctly: {icon:?}"
                );
            }
        }

        Ok(())
    }

    #[test]
    fn repeated_draws_reuse_textures_and_display_sizes_have_separate_entries()
    -> Result<(), Arc<str>> {
        let ctx = egui::Context::default();
        let small = CacheKey {
            icon: Icon::Settings,
            pixels: 18,
        };
        let large = CacheKey {
            icon: Icon::Settings,
            pixels: 36,
        };

        let first = texture(&ctx, small)?;
        ctx.memory_mut(|memory| memory.caches.update());
        let repeated = texture(&ctx, small)?;
        let scaled = texture(&ctx, large)?;
        let original = texture(&ctx, small)?;

        assert_eq!(first.id(), repeated.id());
        assert_eq!(first.id(), original.id());
        assert_ne!(first.id(), scaled.id());
        assert_eq!(scaled.size(), [36, 36]);
        assert_eq!(
            ctx.memory_mut(|memory| memory.caches.cache::<IconCache>().len()),
            2
        );

        Ok(())
    }

    #[test]
    fn texture_cache_stays_bounded_and_releases_unused_icons() -> Result<(), Arc<str>> {
        let ctx = egui::Context::default();

        for pixels in 1..=(MAX_CACHED_TEXTURES as u32 + 5) {
            let _texture = texture(
                &ctx,
                CacheKey {
                    icon: Icon::Check,
                    pixels,
                },
            )?;
        }

        assert_eq!(
            ctx.memory_mut(|memory| memory.caches.cache::<IconCache>().len()),
            MAX_CACHED_TEXTURES
        );

        for _ in 0..=UNUSED_FRAME_LIMIT {
            ctx.memory_mut(|memory| memory.caches.update());
        }

        assert_eq!(
            ctx.memory_mut(|memory| memory.caches.cache::<IconCache>().len()),
            0
        );

        Ok(())
    }
}
