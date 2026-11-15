//! Draws the current subtitle cues over the video
//!
//! Text is laid out at a size relative to the visible picture with a dark outline,
//! stacked up from the bottom or down from the top. Bitmap cues keep their place on
//! the subtitle canvas, stretched over the whole picture

use std::{collections::HashMap, sync::Arc};

use egui::{
    Align, Color32, FontId, Pos2, Rect, TextureHandle, TextureOptions, Vec2,
    text::{LayoutJob, TextFormat},
};

use crate::video::subtitles::{
    ActiveSubtitles, CueContent, SubtitleBitmap, SubtitleText,
};

/// Text height as a fraction of the visible picture's height
const TEXT_SCALE: f32 = 0.048;
const MIN_TEXT: f32 = 15.;
const MAX_TEXT: f32 = 64.;
/// Space between the picture's edge and the nearest line, relative to text size
const EDGE_MARGIN: f32 = 1.2;
const OUTLINE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 220);

/// Textures of the bitmap cues on screen, released when their cues end
#[derive(Default)]
pub struct SubtitleOverlay {
    textures: HashMap<usize, TextureHandle>,
}

impl SubtitleOverlay {
    /// Paints the cues; `picture` is where the whole frame lands, and `visible` the
    /// part of it inside the canvas
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        picture: Rect,
        visible: Rect,
        subtitles: &ActiveSubtitles,
    ) {
        self.release_unused(subtitles);

        if subtitles.is_empty() || !visible.is_positive() {
            return;
        }

        let size = (visible.height() * TEXT_SCALE).clamp(MIN_TEXT, MAX_TEXT);
        let mut bottom = visible.bottom() - size * EDGE_MARGIN;
        let mut top = visible.top() + size * EDGE_MARGIN;

        // Later cues sit closer to the edge, so earlier ones stack outward
        for cue in subtitles.iter().collect::<Vec<_>>().into_iter().rev() {
            match &cue.content {
                CueContent::Text(text) if text.top => {
                    top += paint_text(painter, text, size, visible, Edge::Top(top))
                        + size * 0.2;
                }
                CueContent::Text(text) => {
                    bottom -=
                        paint_text(painter, text, size, visible, Edge::Bottom(bottom))
                            + size * 0.2;
                }
                CueContent::Bitmap(bitmap) => {
                    self.paint_bitmap(painter, bitmap, picture)
                }
            }
        }
    }

    fn paint_bitmap(
        &mut self,
        painter: &egui::Painter,
        bitmap: &SubtitleBitmap,
        picture: Rect,
    ) {
        let key = Arc::as_ptr(&bitmap.image) as usize;
        let texture = self.textures.entry(key).or_insert_with(|| {
            painter.ctx().load_texture(
                format!("subtitle:{key}"),
                (*bitmap.image).clone(),
                TextureOptions::LINEAR,
            )
        });

        let [x, y, width, height] = bitmap.rect;
        let rect = Rect::from_min_size(
            picture.min + Vec2::new(x, y) * picture.size(),
            Vec2::new(width, height) * picture.size(),
        );
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.));

        painter.image(texture.id(), rect, uv, Color32::WHITE);
    }

    fn release_unused(&mut self, subtitles: &ActiveSubtitles) {
        let showing: Vec<usize> = subtitles
            .iter()
            .filter_map(|cue| match &cue.content {
                CueContent::Bitmap(bitmap) => Some(Arc::as_ptr(&bitmap.image) as usize),
                CueContent::Text(_) => None,
            })
            .collect();

        self.textures.retain(|key, _| showing.contains(key));
    }
}

/// Where a block of text attaches: its bottom edge or its top edge
#[derive(Clone, Copy)]
enum Edge {
    Bottom(f32),
    Top(f32),
}

/// Paints centered, outlined text and returns its height
fn paint_text(
    painter: &egui::Painter,
    text: &SubtitleText,
    size: f32,
    visible: Rect,
    edge: Edge,
) -> f32 {
    let galley = painter.layout_job(layout(text, size, visible.width() * 0.9));
    let height = galley.size().y;
    let y = match edge {
        Edge::Bottom(bottom) => bottom - height,
        Edge::Top(top) => top,
    };
    let origin = Pos2::new(visible.center().x, y);
    let outline = (size * 0.07).max(1.);

    for (dx, dy) in [
        (-1., -1.),
        (0., -1.),
        (1., -1.),
        (-1., 0.),
        (1., 0.),
        (-1., 1.),
        (0., 1.),
        (1., 1.),
    ] {
        let offset = Vec2::new(dx, dy) * outline;
        painter.galley_with_override_text_color(
            origin + offset,
            galley.clone(),
            OUTLINE,
        );
    }

    painter.galley(origin, galley, Color32::WHITE);
    height
}

fn layout(text: &SubtitleText, size: f32, width: f32) -> LayoutJob {
    let mut job = LayoutJob {
        halign: Align::Center,
        ..Default::default()
    };
    job.wrap.max_width = width;

    for span in &text.spans {
        let format = TextFormat {
            font_id: FontId::proportional(size),
            color: Color32::WHITE,
            italics: span.italic,
            ..Default::default()
        };
        job.append(&span.text, 0., format);
    }

    job
}
