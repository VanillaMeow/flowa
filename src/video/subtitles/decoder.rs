//! Decodes subtitle packets into timed text and bitmap cues

use std::{ffi::CStr, sync::Arc};

use anyhow::Result;
use egui::{Color32, ColorImage};

use super::{Cue, CueContent, Span, SubtitleBitmap, SubtitleText, parse_ass_event};
use crate::video::{
    ffi::ffmpeg as ffi,
    ffmpeg::{CodecContext, FfmpegDefaults, Packet, check, rational, timestamp},
};

/// What one decoded subtitle packet means for the track
pub(in crate::video) enum SubtitleEvent {
    Cues(Vec<Cue>),
    /// Every cue showing at this time ends, as an empty bitmap update does
    Clear(f64),
}

pub(in crate::video) struct SubtitleDecoder {
    codec: CodecContext,
    time_base: f64,
    origin: f64,
    /// The canvas bitmap positions refer to when the decoder reports none
    fallback_canvas: [u32; 2],
}

/// Owns an `AVSubtitle` and frees its rectangles
struct Decoded(ffi::AVSubtitle);

impl Drop for Decoded {
    fn drop(&mut self) {
        // SAFETY: The decoder filled this subtitle, and nothing borrows it any more
        unsafe { ffi::avsubtitle_free(&mut self.0) };
    }
}

impl SubtitleDecoder {
    /// Opens the stream's decoder; times are media seconds after `origin`
    pub fn open(
        stream: &ffi::AVStream,
        origin: f64,
        fallback_canvas: [u32; 2],
    ) -> Result<Self> {
        Ok(Self {
            codec: CodecContext::open(stream, FfmpegDefaults)?,
            time_base: rational(stream.time_base),
            origin,
            fallback_canvas,
        })
    }

    pub fn flush(&mut self) {
        self.codec.flush();
    }

    pub fn decode(&mut self, packet: &Packet) -> Result<Option<SubtitleEvent>> {
        let mut decoded = Decoded(ffi::AVSubtitle::default());
        let mut got = 0;

        // SAFETY: The codec, subtitle, and borrowed packet are valid for the call
        check(
            unsafe {
                ffi::avcodec_decode_subtitle2(
                    self.codec.raw(),
                    &mut decoded.0,
                    &mut got,
                    packet.raw(),
                )
            },
            "Cannot decode subtitles",
        )?;

        if got == 0 {
            return Ok(None);
        }

        // SAFETY: The packet stays alive for this borrow
        let raw = unsafe { &*packet.raw() };
        let Some((start, end)) = self.times(&decoded.0, raw) else {
            return Ok(None);
        };

        if decoded.0.num_rects == 0 {
            return Ok(Some(SubtitleEvent::Clear(start)));
        }

        let cues = self
            .contents(&decoded.0)
            .into_iter()
            .map(|content| Cue {
                start,
                end,
                content,
            })
            .collect();

        Ok(Some(SubtitleEvent::Cues(cues)))
    }

    /// Start and end in media seconds, with an infinite end when the format leaves it
    /// to the next cue
    fn times(
        &self,
        subtitle: &ffi::AVSubtitle,
        packet: &ffi::AVPacket,
    ) -> Option<(f64, f64)> {
        let microseconds = 1. / f64::from(ffi::AV_TIME_BASE);
        let base = timestamp(subtitle.pts, microseconds, self.origin)
            .or_else(|| timestamp(packet.pts, self.time_base, self.origin))?;
        let start = base + f64::from(subtitle.start_display_time) / 1000.;
        let end = match subtitle.end_display_time {
            0 | u32::MAX if packet.duration > 0 => {
                start + packet.duration as f64 * self.time_base
            }
            0 | u32::MAX => f64::INFINITY,
            end => base + f64::from(end) / 1000.,
        };

        Some((start, end.max(start)))
    }

    fn contents(&self, subtitle: &ffi::AVSubtitle) -> Vec<CueContent> {
        let canvas = self.canvas();
        let count = subtitle.num_rects as usize;

        // SAFETY: The decoder allocated `num_rects` rectangle pointers
        let rects = unsafe { std::slice::from_raw_parts(subtitle.rects, count) };

        rects
            .iter()
            .filter_map(|&rect| {
                // SAFETY: Each pointer refers to a rectangle owned by the subtitle
                let rect = unsafe { &*rect };

                match rect.type_ {
                    ffi::SUBTITLE_ASS => text(rect.ass)
                        .and_then(|line| parse_ass_event(&line).map(CueContent::Text)),
                    ffi::SUBTITLE_TEXT => text(rect.text).map(|plain| {
                        CueContent::Text(SubtitleText {
                            spans: vec![Span {
                                text: plain.trim().to_owned(),
                                italic: false,
                            }],
                            top: false,
                        })
                    }),
                    ffi::SUBTITLE_BITMAP => {
                        bitmap(rect, canvas).map(CueContent::Bitmap)
                    }
                    _ => None,
                }
            })
            .collect()
    }

    fn canvas(&self) -> [u32; 2] {
        let context = self.codec.data();

        if context.width > 0 && context.height > 0 {
            [context.width as u32, context.height as u32]
        } else {
            self.fallback_canvas
        }
    }
}

fn text(pointer: *const std::ffi::c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }

    // SAFETY: FFmpeg terminates subtitle text with NUL
    Some(
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Expands a paletted subtitle bitmap into colors and places it on the canvas
fn bitmap(rect: &ffi::AVSubtitleRect, canvas: [u32; 2]) -> Option<SubtitleBitmap> {
    let (width, height) =
        (usize::try_from(rect.w).ok()?, usize::try_from(rect.h).ok()?);
    let stride = usize::try_from(rect.linesize[0]).ok()?;
    let colors = usize::try_from(rect.nb_colors).ok()?.min(256);

    if width == 0 || height == 0 || rect.data[0].is_null() || rect.data[1].is_null() {
        return None;
    }

    // SAFETY: Bitmap rectangles hold `h` rows of indices and a palette of native-endian
    // 32-bit ARGB
    let (indices, palette) = unsafe {
        (
            std::slice::from_raw_parts(rect.data[0], stride * height),
            std::slice::from_raw_parts(rect.data[1].cast::<u32>(), colors),
        )
    };

    let palette: Vec<Color32> = palette
        .iter()
        .map(|&argb| {
            let channel = |shift: u32| (argb >> shift) as u8;
            Color32::from_rgba_unmultiplied(
                channel(16),
                channel(8),
                channel(0),
                channel(24),
            )
        })
        .collect();
    let pixels = (0..height)
        .flat_map(|row| &indices[row * stride..row * stride + width])
        .map(|&index| {
            palette
                .get(index as usize)
                .copied()
                .unwrap_or(Color32::TRANSPARENT)
        })
        .collect();

    let [canvas_width, canvas_height] = canvas.map(|side| side.max(1) as f32);

    Some(SubtitleBitmap {
        image: Arc::new(ColorImage::new([width, height], pixels)),
        rect: [
            rect.x as f32 / canvas_width,
            rect.y as f32 / canvas_height,
            width as f32 / canvas_width,
            height as f32 / canvas_height,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmaps_expand_their_palette_and_place_themselves_on_the_canvas() {
        // Two rows of three indices, padded to a stride of four
        let mut indices = [0_u8, 1, 1, 9, 1, 0, 0, 9];
        let mut palette = [0x0000_0000_u32, 0x80FF_2010];
        let mut rect = ffi::AVSubtitleRect {
            x: 100,
            y: 400,
            w: 3,
            h: 2,
            nb_colors: 2,
            type_: ffi::SUBTITLE_BITMAP,
            ..Default::default()
        };
        rect.data[0] = indices.as_mut_ptr();
        rect.data[1] = palette.as_mut_ptr().cast();
        rect.linesize[0] = 4;

        let bitmap = bitmap(&rect, [800, 500]).expect("A bitmap");

        assert_eq!(bitmap.image.size, [3, 2]);
        assert_eq!(bitmap.rect, [0.125, 0.8, 3. / 800., 2. / 500.]);
        assert_eq!(bitmap.image.pixels[0], Color32::TRANSPARENT);
        assert_eq!(
            bitmap.image.pixels[1],
            Color32::from_rgba_unmultiplied(0xFF, 0x20, 0x10, 0x80)
        );
        assert_eq!(bitmap.image.pixels[3], bitmap.image.pixels[1]);
    }
}
