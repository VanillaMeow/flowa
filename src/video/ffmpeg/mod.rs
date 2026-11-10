//! Backend-neutral ownership of FFmpeg objects, shared by every part of video

use std::{ffi::CStr, path::Path};

use anyhow::{Result, ensure};

use super::ffi::ffmpeg as ffi;

mod codec_context;
mod frame;
mod hw_device;
mod input;
mod packet;
mod stream;

pub(super) use codec_context::{CodecContext, CodecSetup, FfmpegDefaults};
pub(super) use frame::{AvFrame, FrameInfo};
pub(super) use hw_device::HwDevice;
pub(super) use input::{Input, Probe};
pub(super) use packet::{Packet, PacketKind};
pub(super) use stream::{Stream, StreamInfo};

/// What a codec returned when asked for its next output
pub(super) enum Receive<T> {
    Again,
    Item(T),
    End,
}

/// What a codec did with an offered packet
pub(super) enum Sent {
    Accepted,
    Again,
    End,
}

pub(super) fn check(code: i32, operation: &str) -> Result<i32> {
    if code >= 0 {
        return Ok(code);
    }

    let mut buffer = [0; 64];

    // SAFETY: FFmpeg writes a bounded NUL-terminated error into this buffer
    unsafe { ffi::av_strerror(code, buffer.as_mut_ptr(), buffer.len()) };
    let message = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
    anyhow::bail!("{operation}: {message}")
}

pub(super) fn rational(value: ffi::AVRational) -> f64 {
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

pub(super) fn stream_rotation(stream: &ffi::AVStream) -> Option<f64> {
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

pub(super) fn dimensions(width: i32, height: i32) -> Result<()> {
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
