//! Timed subtitle cues, the formatting kept from them, and the cues to show now
//!
//! FFmpeg turns every text format into ASS event lines; their override tags are
//! reduced to italics, top placement, and skipped vector drawings. Bitmap formats
//! such as PGS keep their images and position on the subtitle canvas

// Builds without playback never create cues but still draw the state's empty list
#![cfg_attr(not(video_playback), allow(dead_code))]

#[cfg(video_playback)]
mod decoder;
#[cfg(video_playback)]
mod external;

use std::sync::Arc;

#[cfg(video_playback)]
pub(super) use decoder::{SubtitleDecoder, SubtitleEvent};
#[cfg(video_playback)]
pub(super) use external::{ExternalSubtitles, load_external};

/// Bitmap cues further than this from the playback position are dropped
const BITMAP_RETENTION: f64 = 120.;
/// Cues starting this close together count as the same cue when read again
const SAME_START: f64 = 0.001;

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// Media seconds when the cue appears
    pub start: f64,
    /// Media seconds when it disappears, or infinity to last until the next cue
    pub end: f64,
    pub content: CueContent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CueContent {
    Text(SubtitleText),
    Bitmap(SubtitleBitmap),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubtitleText {
    /// Runs of text with one style each; line breaks are `\n`
    pub spans: Vec<Span>,
    /// Placed at the top of the picture instead of the bottom
    pub top: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub italic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubtitleBitmap {
    pub image: Arc<egui::ColorImage>,
    /// Left, top, width, and height as fractions of the subtitle canvas
    pub rect: [f32; 4],
}

/// The cues to draw at one moment, compared by identity so snapshots stay cheap
#[derive(Clone, Debug, Default)]
pub struct ActiveSubtitles(Vec<Arc<Cue>>);

impl ActiveSubtitles {
    pub fn iter(&self) -> impl Iterator<Item = &Cue> {
        self.0.iter().map(Arc::as_ref)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl PartialEq for ActiveSubtitles {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(&other.0).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

/// One track's cues in start order
#[derive(Clone, Debug, Default)]
pub struct SubtitleCues {
    cues: Vec<Arc<Cue>>,
}

impl SubtitleCues {
    pub fn len(&self) -> usize {
        self.cues.len()
    }

    /// Adds a cue unless the same one was read before a seek
    pub fn insert(&mut self, cue: Cue) {
        let duplicate = self.cues.iter().any(|existing| {
            (existing.start - cue.start).abs() < SAME_START
                && existing.content == cue.content
        });

        if duplicate {
            return;
        }

        let index = self
            .cues
            .partition_point(|existing| existing.start <= cue.start);
        self.cues.insert(index, Arc::new(cue));
    }

    /// Ends every cue still showing at `time`, as an empty bitmap event does
    pub fn clear_at(&mut self, time: f64) {
        for cue in &mut self.cues {
            if cue.start < time && cue.end > time {
                Arc::make_mut(cue).end = time;
            }
        }
    }

    /// Drops bitmap cues far from the position, keeping text, which is small
    pub fn prune(&mut self, position: f64) {
        self.cues.retain(|cue| {
            let near = cue.start < position + BITMAP_RETENTION
                && cue.end.min(cue.start + BITMAP_RETENTION)
                    > position - BITMAP_RETENTION;

            matches!(cue.content, CueContent::Text(_)) || near
        });
    }

    /// The cues showing at `position`; an open-ended cue lasts until the next one
    pub fn active(&self, position: f64) -> ActiveSubtitles {
        let showing = self.cues.iter().enumerate().filter(|(index, cue)| {
            let next = self
                .cues
                .get(index + 1)
                .map_or(f64::INFINITY, |next| next.start);
            let end = if cue.end.is_finite() { cue.end } else { next };

            cue.start <= position && position < end
        });

        ActiveSubtitles(showing.map(|(_, cue)| Arc::clone(cue)).collect())
    }
}

/// The text of an ASS event line such as FFmpeg's decoders produce:
/// `ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text`
pub fn parse_ass_event(line: &str) -> Option<SubtitleText> {
    let text = line.splitn(9, ',').nth(8)?;

    parse_ass_text(text)
}

/// Keeps text and italics from ASS markup, places `\an7`–`\an9` at the top, and skips
/// vector drawings
pub fn parse_ass_text(markup: &str) -> Option<SubtitleText> {
    let mut parsed = SubtitleText::default();
    let mut style = TextStyle::default();
    let mut current = String::new();
    let mut characters = markup.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '{' => {
                let block: String =
                    characters.by_ref().take_while(|&c| c != '}').collect();
                let mut next = style;
                apply_overrides(&block, &mut next, &mut parsed.top);

                if next.italic != style.italic {
                    push_span(&mut parsed.spans, &mut current, style.italic);
                }
                style = next;
            }
            '\\' => match characters.peek() {
                Some('N' | 'n') => {
                    characters.next();
                    if !style.drawing {
                        current.push('\n');
                    }
                }
                Some('h') => {
                    characters.next();
                    if !style.drawing {
                        current.push('\u{a0}');
                    }
                }
                _ if !style.drawing => current.push('\\'),
                _ => {}
            },
            _ if !style.drawing => current.push(character),
            _ => {}
        }
    }

    push_span(&mut parsed.spans, &mut current, style.italic);
    trim_spans(&mut parsed.spans);

    (!parsed.spans.is_empty()).then_some(parsed)
}

#[derive(Clone, Copy, Default)]
struct TextStyle {
    italic: bool,
    /// Inside `\p1` or higher, where text is vector drawing commands
    drawing: bool,
}

fn apply_overrides(block: &str, style: &mut TextStyle, top: &mut bool) {
    for tag in block.split('\\').map(str::trim) {
        if let Some(value) = tag.strip_prefix("an") {
            *top = matches!(value, "7" | "8" | "9");
        } else if let Some(value) = tag.strip_prefix('i')
            && matches!(value, "" | "0" | "1")
        {
            style.italic = value == "1";
        } else if let Some(value) = tag.strip_prefix('p')
            && let Ok(scale) = value.parse::<u32>()
        {
            style.drawing = scale > 0;
        }
    }
}

fn push_span(spans: &mut Vec<Span>, current: &mut String, italic: bool) {
    if current.is_empty() {
        return;
    }

    spans.push(Span {
        text: std::mem::take(current),
        italic,
    });
}

/// Removes surrounding whitespace and blank lines from the whole cue
fn trim_spans(spans: &mut Vec<Span>) {
    if let Some(first) = spans.first_mut() {
        first.text = first.text.trim_start().to_owned();
    }
    if let Some(last) = spans.last_mut() {
        last.text = last.text.trim_end().to_owned();
    }

    spans.retain(|span| !span.text.is_empty());
}
