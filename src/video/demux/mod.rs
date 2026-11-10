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
    AudioInfo,
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
    pub info: StreamInfo,
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

        Ok(Self {
            input,
            video_index,
            audio_index,
            info,
        })
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
    let audio = audio_index.map(|index| {
        // SAFETY: The input owns the selected stream and its codec parameters
        let parameters = unsafe { &*input.stream(index).codecpar };
        AudioInfo {
            codec: codec_name(parameters.codec_id),
            channels: parameters.ch_layout.nb_channels.max(0) as u32,
            sample_rate: parameters.sample_rate.max(0) as u32,
            bit_rate: parameters.bit_rate.max(0) as u64,
        }
    });

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
