//! Subtitle files next to the video, such as `Movie.en.srt` beside `Movie.mkv`

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{SubtitleCues, SubtitleDecoder, SubtitleEvent};
use crate::video::{
    Track,
    demux::{format_name, language_name},
    ffi::ffmpeg as ffi,
    ffmpeg::{Input, Probe},
};

const EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt"];

/// A whole subtitle file, read before it is shown
pub(in crate::video) struct ExternalSubtitles {
    pub track: Track,
    pub cues: SubtitleCues,
}

/// Reads every subtitle file named after the video; their track indices continue
/// after the container's streams
pub(in crate::video) fn load_external(
    video: &Path,
    first_index: usize,
    canvas: [u32; 2],
    cancel: impl Fn() -> bool + Clone + Send + Sync + 'static,
) -> Vec<ExternalSubtitles> {
    sidecars(video)
        .into_iter()
        .enumerate()
        .filter_map(|(offset, (path, qualifier))| {
            let index = first_index + offset;

            match read(&path, canvas, cancel.clone()) {
                Ok(cues) if cues.len() > 0 => Some(ExternalSubtitles {
                    track: track(&path, &qualifier, index),
                    cues,
                }),
                Ok(_) => None,
                Err(error) => {
                    log::warn!("Cannot read subtitles {}: {error:#}", path.display());
                    None
                }
            }
        })
        .collect()
}

/// Subtitle files whose names start with the video's name, with whatever sits
/// between the name and the extension, such as a language
fn sidecars(video: &Path) -> Vec<(PathBuf, String)> {
    let (Some(folder), Some(stem)) = (video.parent(), video.file_stem()) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let stem = stem.to_string_lossy();

    let mut found: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
            if !EXTENSIONS.contains(&extension.as_str()) {
                return None;
            }

            let name = path.file_stem()?.to_string_lossy().into_owned();
            let qualifier = if name == *stem {
                String::new()
            } else {
                name.strip_prefix(&*stem)?.strip_prefix('.')?.to_owned()
            };

            Some((path, qualifier))
        })
        .collect();

    found.sort_by(|a, b| {
        natord::compare(&a.0.to_string_lossy(), &b.0.to_string_lossy())
    });
    found
}

fn read(
    path: &Path,
    canvas: [u32; 2],
    cancel: impl Fn() -> bool + Send + Sync + 'static,
) -> Result<SubtitleCues> {
    let mut input = Input::open(path, cancel, Probe::Playback)?;
    let index = input.best_stream(ffi::AVMEDIA_TYPE_SUBTITLE)?;
    let mut decoder = SubtitleDecoder::open(input.stream(index), 0., canvas)?;
    let mut cues = SubtitleCues::default();

    while let Some(packet) = input.read()? {
        // SAFETY: The packet was just read and is owned here
        let stream = unsafe { (*packet.raw()).stream_index };
        if usize::try_from(stream) != Ok(index) {
            continue;
        }

        match decoder.decode(&packet)? {
            Some(SubtitleEvent::Cues(decoded)) => {
                decoded.into_iter().for_each(|cue| cues.insert(cue))
            }
            Some(SubtitleEvent::Clear(time)) => cues.clear_at(time),
            None => {}
        }
    }

    Ok(cues)
}

/// Labels a file from its qualifier, such as `English · SRT · External`
fn track(path: &Path, qualifier: &str, index: usize) -> Track {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let format = match extension.as_str() {
        "srt" => "subrip",
        "vtt" => "webvtt",
        other => other,
    };
    let lowercase = qualifier.to_ascii_lowercase();
    let forced = lowercase.split('.').any(|part| part == "forced");
    let language = qualifier
        .split('.')
        .find(|part| !part.is_empty() && !part.eq_ignore_ascii_case("forced"))
        .map(language_name);

    let label = match language {
        Some(language) => format!("{language} · {} · External", format_name(format)),
        None => format!("{} · External", format_name(format)),
    };

    Track {
        index,
        label,
        default: false,
        forced,
        external: true,
    }
}
