//! Opens a media file, picks its video and audio streams, and reads and seeks packets

use std::{
    ffi::CStr,
    path::Path,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Result;

use super::{
    AudioInfo, Track, Tracks,
    ffi::ffmpeg as ffi,
    ffmpeg::{
        Input, Packet, PacketKind, Probe, Stream, StreamInfo, check, rational,
        stream_rotation,
    },
};

/// Interrupts blocking reads when playback stops or a newer seek arrives
#[derive(Default)]
pub(super) struct InterruptFlags {
    pub abort: AtomicBool,
    pub seeking: AtomicBool,
}

pub(super) struct Media {
    input: Input<'static>,
    video_index: usize,
    audio_index: Option<usize>,
    subtitle_index: Option<usize>,
    pub info: StreamInfo,
    pub tracks: Tracks,
}

impl Media {
    pub fn open(path: &Path, interrupt: Arc<InterruptFlags>) -> Result<Self> {
        let input = Input::open(
            path,
            move || {
                interrupt.abort.load(Ordering::SeqCst)
                    || interrupt.seeking.load(Ordering::SeqCst)
            },
            Probe::Playback,
        )?;
        let video_index = input.best_stream(ffi::AVMEDIA_TYPE_VIDEO)?;
        let audio_index = input.best_stream(ffi::AVMEDIA_TYPE_AUDIO).ok();
        let info = stream_info(&input, video_index, audio_index);
        let tracks = Tracks {
            audio: tracks(&input, ffi::AVMEDIA_TYPE_AUDIO),
            subtitles: tracks(&input, ffi::AVMEDIA_TYPE_SUBTITLE),
        };

        Ok(Self {
            input,
            video_index,
            audio_index,
            subtitle_index: None,
            info,
            tracks,
        })
    }

    /// The coded picture size, which bitmap subtitles fall back to as their canvas
    pub fn video_size(&self) -> [u32; 2] {
        // SAFETY: The input owns the video stream and its codec parameters
        let parameters = unsafe { &*self.input.stream(self.video_index).codecpar };

        [parameters.width, parameters.height].map(|side| side.max(0) as u32)
    }

    pub fn stream_count(&self) -> usize {
        self.input.data().nb_streams as usize
    }

    pub fn subtitle_index(&self) -> Option<usize> {
        self.subtitle_index
    }

    /// Reports the embedded subtitle stream's packets as subtitles, or none
    pub fn select_subtitles(&mut self, index: Option<usize>) -> Result<()> {
        if let Some(index) = index {
            anyhow::ensure!(
                self.tracks
                    .subtitles
                    .iter()
                    .any(|track| track.index == index && !track.external),
                "The video has no subtitle track {index}"
            );
        }

        self.subtitle_index = index;
        Ok(())
    }

    pub fn subtitle_stream(&self) -> Option<&ffi::AVStream> {
        self.subtitle_index.map(|index| self.input.stream(index))
    }

    pub fn audio_index(&self) -> Option<usize> {
        self.audio_index
    }

    /// Reads another audio stream from the next packet on; the caller seeks and opens
    /// its decoder
    pub fn select_audio(&mut self, index: usize) -> Result<()> {
        anyhow::ensure!(
            self.tracks.audio.iter().any(|track| track.index == index),
            "The video has no audio track {index}"
        );

        self.audio_index = Some(index);
        self.info.audio = Some(audio_info(self.input.stream(index)));
        self.info.audio_time_base = rational(self.input.stream(index).time_base);

        Ok(())
    }

    /// An independent copy of the video stream for decoders on other threads
    pub fn copy_stream(&self) -> Result<Arc<Stream>> {
        Ok(Arc::new(Stream::copy(
            self.input.stream(self.video_index),
            self.info,
        )?))
    }

    /// The selected audio stream, for opening its decoder
    pub fn audio_stream(&self) -> Option<&ffi::AVStream> {
        self.audio_index.map(|index| self.input.stream(index))
    }

    pub fn read(&mut self) -> Result<Option<Packet>> {
        let Some(mut packet) = self.input.read()? else {
            return Ok(None);
        };
        let index = usize::try_from(unsafe { (*packet.raw()).stream_index })?;

        packet.kind = if index == self.video_index {
            PacketKind::Video
        } else if Some(index) == self.audio_index {
            PacketKind::Audio
        } else if Some(index) == self.subtitle_index {
            PacketKind::Subtitle
        } else {
            PacketKind::Other
        };

        Ok(Some(packet))
    }

    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        let timestamp = ((seconds + self.info.origin) * 1_000_000.0).round() as i64;

        self.input.clear_interrupted_io();

        // SAFETY: Only the demux worker accesses this input
        check(
            unsafe {
                ffi::avformat_seek_file(
                    self.input.raw(),
                    -1,
                    i64::MIN,
                    timestamp,
                    timestamp,
                    ffi::AVSEEK_FLAG_BACKWARD as i32,
                )
            },
            "Cannot seek",
        )?;

        self.input.clear_interrupted_io();

        Ok(())
    }
}

fn stream_info(
    input: &Input<'_>,
    video_index: usize,
    audio_index: Option<usize>,
) -> StreamInfo {
    let format = input.data();
    let video = input.stream(video_index);

    // SAFETY: The context owns this stream and codec parameters
    let rate = unsafe {
        ffi::av_guess_frame_rate(
            input.raw(),
            video as *const _ as *mut _,
            ptr::null_mut(),
        )
    };
    let codec = codec_name(unsafe { (*video.codecpar).codec_id });
    let audio = audio_index.map(|index| audio_info(input.stream(index)));

    let video_time_base = rational(video.time_base);
    let mut duration = if format.duration == i64::MIN {
        0.0
    } else {
        format.duration as f64 / f64::from(ffi::AV_TIME_BASE)
    };

    if duration <= 0.0 && video.duration != i64::MIN {
        duration = video.duration as f64 * video_time_base;
    }

    StreamInfo {
        origin: if format.start_time == i64::MIN {
            0.0
        } else {
            format.start_time as f64 / f64::from(ffi::AV_TIME_BASE)
        },
        duration,
        video_time_base,
        audio_time_base: audio_index
            .map_or(0.0, |index| rational(input.stream(index).time_base)),
        frame_duration: if rate.num > 0 && rate.den > 0 {
            f64::from(rate.den) / f64::from(rate.num)
        } else {
            1.0 / 30.0
        },
        rotation: stream_rotation(video).unwrap_or(0.0),
        audio,
        codec,
    }
}

fn audio_info(stream: &ffi::AVStream) -> AudioInfo {
    // SAFETY: The input owns the stream and its codec parameters
    let parameters = unsafe { &*stream.codecpar };

    AudioInfo {
        codec: codec_name(parameters.codec_id),
        channels: parameters.ch_layout.nb_channels.max(0) as u32,
        sample_rate: parameters.sample_rate.max(0) as u32,
        bit_rate: parameters.bit_rate.max(0) as u64,
    }
}

/// Every stream of one kind, labeled for the track menus
fn tracks(input: &Input<'_>, kind: ffi::AVMediaType) -> Vec<Track> {
    let count = input.data().nb_streams as usize;
    let mut tracks = Vec::new();

    for index in 0..count {
        let stream = input.stream(index);

        // SAFETY: The input owns the stream and its codec parameters
        let parameters = unsafe { &*stream.codecpar };
        if parameters.codec_type != kind {
            continue;
        }

        let number = tracks.len() + 1;
        tracks.push(Track {
            index,
            label: track_label(stream, parameters, number),
            default: stream.disposition & ffi::AV_DISPOSITION_DEFAULT as i32 != 0,
            forced: stream.disposition & ffi::AV_DISPOSITION_FORCED as i32 != 0,
            external: false,
        });
    }

    tracks
}

/// Joins the title, language, and format, such as `Commentary · English · AAC stereo`
fn track_label(
    stream: &ffi::AVStream,
    parameters: &ffi::AVCodecParameters,
    number: usize,
) -> String {
    let title = metadata(stream, c"title");
    let language = metadata(stream, c"language").map(|code| language_name(&code));
    let format = match parameters.codec_type {
        ffi::AVMEDIA_TYPE_AUDIO => format!(
            "{} {}",
            format_name(codec_name(parameters.codec_id)),
            channel_description(parameters.ch_layout.nb_channels)
        ),
        _ => format_name(codec_name(parameters.codec_id)).to_owned(),
    };

    let parts: Vec<String> = [title, language].into_iter().flatten().collect();

    if parts.is_empty() {
        return format!("Track {number} · {format}");
    }

    format!("{} · {format}", parts.join(" · "))
}

fn metadata(stream: &ffi::AVStream, key: &CStr) -> Option<String> {
    // SAFETY: The stream owns its dictionary, and FFmpeg returns a borrowed entry
    let entry =
        unsafe { ffi::av_dict_get(stream.metadata, key.as_ptr(), ptr::null(), 0) };

    if entry.is_null() {
        return None;
    }

    // SAFETY: Dictionary values are NUL-terminated strings owned by the stream
    let value = unsafe { CStr::from_ptr((*entry).value) }.to_string_lossy();
    let value = value.trim();

    (!value.is_empty() && value != "und").then(|| value.to_owned())
}

/// English names for common ISO 639-2 codes, or the code itself
pub(super) fn language_name(code: &str) -> String {
    let name = match code.to_ascii_lowercase().as_str() {
        "eng" | "en" => "English",
        "jpn" | "ja" => "Japanese",
        "ger" | "deu" | "de" => "German",
        "fre" | "fra" | "fr" => "French",
        "spa" | "es" => "Spanish",
        "ita" | "it" => "Italian",
        "por" | "pt" => "Portuguese",
        "rus" | "ru" => "Russian",
        "chi" | "zho" | "zh" => "Chinese",
        "kor" | "ko" => "Korean",
        "dut" | "nld" | "nl" => "Dutch",
        "pol" | "pl" => "Polish",
        "swe" | "sv" => "Swedish",
        "nor" | "no" => "Norwegian",
        "dan" | "da" => "Danish",
        "fin" | "fi" => "Finnish",
        "ara" | "ar" => "Arabic",
        "hin" | "hi" => "Hindi",
        "tur" | "tr" => "Turkish",
        "ukr" | "uk" => "Ukrainian",
        _ => return code.to_owned(),
    };

    name.to_owned()
}

pub(super) fn format_name(codec: &str) -> &str {
    match codec {
        "aac" => "AAC",
        "ac3" => "AC-3",
        "eac3" => "E-AC-3",
        "dts" => "DTS",
        "truehd" => "TrueHD",
        "flac" => "FLAC",
        "opus" => "Opus",
        "vorbis" => "Vorbis",
        "mp3" => "MP3",
        "subrip" => "SRT",
        "ass" | "ssa" => "ASS",
        "webvtt" => "WebVTT",
        "mov_text" => "Timed text",
        "hdmv_pgs_subtitle" => "PGS",
        "dvd_subtitle" => "VobSub",
        "dvb_subtitle" => "DVB",
        other if other.starts_with("pcm_") => "PCM",
        other => other,
    }
}

fn channel_description(channels: i32) -> String {
    match channels {
        1 => "mono".into(),
        2 => "stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        other => format!("{other} channels"),
    }
}

fn codec_name(id: ffi::AVCodecID) -> &'static str {
    // SAFETY: FFmpeg returns a static NUL-terminated name for every codec ID
    unsafe { CStr::from_ptr(ffi::avcodec_get_name(id)) }
        .to_str()
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use anyhow::ensure;

    use super::*;

    #[test]
    #[ignore = "Requires ffmpeg to generate a seekable raw-video fixture"]
    fn seek_recovers_after_an_interrupted_avio_read() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("interrupted.nut");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=128x96:rate=10",
                "-t",
                "3",
                "-c:v",
                "rawvideo",
            ])
            .arg(&path)
            .status()?;
        ensure!(generated.success(), "Cannot generate the seek fixture");

        let mut media = Media::open(&path, Arc::new(InterruptFlags::default()))?;

        // Model the AVIO state left by FFmpeg when a newer seek interrupts a read
        // SAFETY: This test exclusively owns the input and its initialized I/O context
        unsafe {
            let io = (*media.input.raw()).pb;
            ensure!(!io.is_null(), "No AVIO context was opened");
            (*io).error = ffi::FLOWA_AVERROR_EXIT;
            (*io).eof_reached = 1;
        }

        media.seek(1.5)?;
        let mut packets = 0;

        while media.read()?.is_some() {
            packets += 1;
        }
        ensure!(packets > 0, "Seeking did not resume packet reads");

        Ok(())
    }
}
