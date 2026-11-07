//! GPU-independent FFmpeg ownership shared by playback and poster extraction

use std::{ffi::CStr, path::Path};

use anyhow::{Result, ensure};

use super::{AudioInfo, ffmpeg as ffi};

mod audio;
mod codec;
mod frame;
mod input;
mod packet;
mod poster;
mod stream;

pub(super) use audio::AudioBlock;
pub(super) use codec::{Codec, DeviceRef, Threads};
pub(super) use frame::{AvFrame, CpuFrame, FrameInfo};
pub(super) use input::{InterruptFlags, Media};
pub(super) use packet::{Packet, PacketKind};
pub(super) use poster::extract_poster;
pub(super) use stream::Stream;

#[derive(Clone, Copy)]
pub(super) struct StreamInfo {
    pub origin: f64,
    pub duration: f64,
    pub video_time_base: f64,
    pub audio_time_base: f64,
    pub frame_duration: f64,
    pub rotation: f64,
    pub audio: Option<AudioInfo>,
    pub codec: &'static str,
}

pub(super) enum Receive<T> {
    Again,
    Item(T),
    End,
}

pub(super) enum Sent {
    Accepted,
    Again,
    End,
}

fn check(code: i32, operation: &str) -> Result<i32> {
    if code >= 0 {
        return Ok(code);
    }

    let mut buffer = [0; 64];

    // SAFETY: FFmpeg writes a bounded NUL-terminated error into this buffer
    unsafe { ffi::av_strerror(code, buffer.as_mut_ptr(), buffer.len()) };
    let message = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
    anyhow::bail!("{operation}: {message}")
}

fn rational(value: ffi::AVRational) -> f64 {
    f64::from(value.num) / f64::from(value.den)
}

fn path_string(path: &Path) -> Result<std::ffi::CString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        Ok(std::ffi::CString::new(path.as_os_str().as_bytes())?)
    }

    #[cfg(windows)]
    {
        let text = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("The media path is not valid Unicode"))?;

        Ok(std::ffi::CString::new(text)?)
    }
}

pub(super) fn timestamp(value: i64, time_base: f64, origin: f64) -> Option<f64> {
    (value != i64::MIN).then_some(value as f64 * time_base - origin)
}

fn stream_rotation(stream: &ffi::AVStream) -> Option<f64> {
    // SAFETY: The stream owns its codec parameters and side data
    let parameters = unsafe { &*stream.codecpar };
    let side_data = unsafe {
        ffi::av_packet_side_data_get(
            parameters.coded_side_data,
            parameters.nb_coded_side_data,
            ffi::AV_PKT_DATA_DISPLAYMATRIX,
        )
    };

    if side_data.is_null() || unsafe { (*side_data).size } < 9 * size_of::<i32>() {
        return None;
    }

    // SAFETY: DISPLAYMATRIX side data contains an aligned nine-element matrix
    Some(unsafe { ffi::av_display_rotation_get((*side_data).data.cast()) })
}

fn dimensions(width: i32, height: i32) -> Result<()> {
    ensure!(width > 0 && height > 0, "Invalid decoded frame dimensions");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_paths_preserve_unicode_and_reject_embedded_nuls() -> Result<()> {
        let name = "clip-ž-日本語.mp4";
        assert_eq!(path_string(Path::new(name))?.to_bytes(), name.as_bytes());
        assert!(path_string(Path::new("clip\0.mp4")).is_err());

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_media_paths_preserve_non_unicode_bytes() -> Result<()> {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let bytes = b"clip-\xff.mp4";
        assert_eq!(
            path_string(Path::new(OsStr::from_bytes(bytes)))?.to_bytes(),
            bytes
        );

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_media_paths_reject_unpaired_surrogates() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};

        let name = OsString::from_wide(&[0xd800]);
        assert!(path_string(Path::new(&name)).is_err());
    }
}
