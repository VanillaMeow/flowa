//! Demuxer I/O with owned interruption storage and audio decoding

use std::{
    ffi::{CStr, c_void},
    path::Path,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, ensure};

use super::{
    AudioBlock, AudioStreamInfo, Packet, PacketKind, Receive, Sent, Stream, StreamInfo,
    audio::AudioDecoder, check, ffi, path_string, rational, stream_rotation,
};

#[derive(Default)]
pub(in crate::video) struct InterruptFlags {
    pub abort: AtomicBool,
    pub seeking: AtomicBool,
}

struct Interrupt<'a> {
    check: Box<dyn Fn() -> bool + Send + Sync + 'a>,
}

unsafe extern "C" fn interrupted(data: *mut c_void) -> i32 {
    // SAFETY: Input retains this boxed callback until avformat_close_input returns
    let interrupt = unsafe { &*data.cast::<Interrupt<'_>>() };

    i32::from((interrupt.check)())
}

pub(super) enum Probe {
    Playback,
    Poster,
}

pub(super) struct Input<'a> {
    handle: *mut ffi::AVFormatContext,
    _interrupt: Box<Interrupt<'a>>,
}

impl<'a> Input<'a> {
    pub fn open(
        path: &Path,
        cancel: impl Fn() -> bool + Send + Sync + 'a,
        probe: Probe,
    ) -> Result<Self> {
        let path = path_string(path)?;
        let mut interrupt = Box::new(Interrupt {
            check: Box::new(cancel),
        });

        // SAFETY: The new context has a stable, retained interruption callback
        let handle = unsafe { ffi::avformat_alloc_context() };
        ensure!(
            !handle.is_null(),
            "Cannot allocate the FFmpeg media context"
        );

        unsafe {
            (*handle).interrupt_callback = ffi::AVIOInterruptCB {
                callback: Some(interrupted),
                opaque: std::ptr::from_mut(&mut *interrupt).cast(),
            };

            if matches!(probe, Probe::Poster) {
                (*handle).probesize = 2 * 1024 * 1024;
                (*handle).max_analyze_duration = i64::from(ffi::AV_TIME_BASE);
            }
        }

        let mut input = Self {
            handle,
            _interrupt: interrupt,
        };

        // SAFETY: FFmpeg may replace or free the context; Drop always sees the updated pointer
        unsafe {
            check(
                ffi::avformat_open_input(
                    &mut input.handle,
                    path.as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                ),
                "Cannot open media",
            )?;
            check(
                ffi::avformat_find_stream_info(input.handle, ptr::null_mut()),
                "Cannot probe media streams",
            )?;
        }

        Ok(input)
    }

    pub fn raw(&self) -> *mut ffi::AVFormatContext {
        self.handle
    }

    pub fn data(&self) -> &ffi::AVFormatContext {
        // SAFETY: Successful opening guarantees a live context retained by this wrapper
        unsafe { &*self.handle }
    }

    pub fn stream(&self, index: usize) -> &ffi::AVStream {
        assert!(
            index < self.data().nb_streams as usize,
            "Invalid stream index"
        );

        // SAFETY: The context owns the validated stream array and the selected stream
        unsafe { &**self.data().streams.add(index) }
    }

    pub fn best_stream(&self, kind: ffi::AVMediaType) -> Result<usize> {
        // SAFETY: Stream discovery only borrows this live demuxer
        let index = unsafe {
            ffi::av_find_best_stream(self.handle, kind, -1, -1, ptr::null_mut(), 0)
        };

        Ok(check(index, "Cannot find a media stream")? as usize)
    }

    pub fn read(&mut self) -> Result<Option<Packet>> {
        let mut packet = Packet::allocate()?;

        // SAFETY: The demuxer has one owner and the packet is writable
        let result = unsafe { ffi::av_read_frame(self.handle, packet.raw()) };
        if result == ffi::FLOWA_AVERROR_EOF {
            return Ok(None);
        }
        check(result, "Cannot read media")?;

        packet.bytes = usize::try_from(unsafe { (*packet.raw()).size })?;

        Ok(Some(packet))
    }

    fn clear_interrupted_io(&mut self) {
        // FFmpeg retains AVERROR_EXIT in AVIO even after a successful buffered seek
        // SAFETY: Only the demux worker accesses this context and its I/O state
        unsafe {
            let io = (*self.handle).pb;

            if !io.is_null() && (*io).error == ffi::FLOWA_AVERROR_EXIT {
                (*io).error = 0;
                (*io).eof_reached = 0;
            }
        }
    }

    pub fn info(&self, video_index: usize, audio_index: Option<usize>) -> StreamInfo {
        let format = self.data();
        let video = self.stream(video_index);

        // SAFETY: The context owns this stream and codec parameters
        let rate = unsafe {
            ffi::av_guess_frame_rate(
                self.handle,
                video as *const _ as *mut _,
                ptr::null_mut(),
            )
        };
        let codec = codec_name(unsafe { (*video.codecpar).codec_id });
        let audio = audio_index.map(|index| {
            // SAFETY: The input owns the selected stream and its codec parameters
            let parameters = unsafe { &*self.stream(index).codecpar };
            AudioStreamInfo {
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
                .map_or(0.0, |index| rational(self.stream(index).time_base)),
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
}

fn codec_name(id: ffi::AVCodecID) -> [std::ffi::c_char; 64] {
    let mut codec = [0; 64];
    // SAFETY: FFmpeg returns a static NUL-terminated codec name
    let name = unsafe { CStr::from_ptr(ffi::avcodec_get_name(id)) };
    for (destination, &source) in codec.iter_mut().take(63).zip(name.to_bytes()) {
        *destination = source as std::ffi::c_char;
    }
    codec
}

impl Drop for Input<'_> {
    fn drop(&mut self) {
        // SAFETY: The interruption allocation remains alive until closing I/O completes
        unsafe { ffi::avformat_close_input(&mut self.handle) };
    }
}

pub(in crate::video) struct Media {
    audio: Option<AudioDecoder>,
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
        let audio = audio_index
            .map(|index| AudioDecoder::open(input.stream(index)))
            .transpose()?;
        let info = input.info(video_index, audio_index);

        Ok(Self {
            input,
            audio,
            video_index,
            audio_index,
            info,
        })
    }

    pub fn copy_stream(&self) -> Result<Arc<Stream>> {
        Ok(Arc::new(Stream::copy(
            self.input.stream(self.video_index),
            self.info,
        )?))
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

        // SAFETY: Only the demux worker accesses this input and its audio codec
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

        if let Some(audio) = &mut self.audio {
            audio.flush()?;
        }

        Ok(())
    }

    pub fn send_audio(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        self.audio
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No audio decoder is open"))?
            .send(packet)
    }

    pub fn receive_audio(&mut self) -> Result<Receive<AudioBlock>> {
        self.audio
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No audio decoder is open"))?
            .receive()
    }
}

#[cfg(test)]
mod tests {
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
