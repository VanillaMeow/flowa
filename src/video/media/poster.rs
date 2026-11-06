//! Bounded poster decoding and scaling without audio output or GPU initialization

use std::{path::Path, ptr, ptr::NonNull};

use anyhow::{Context, Result, ensure};

use super::{
    Codec, DecodedFrame, Receive, Sent, check, dimensions, ffi,
    input::{Input, Probe},
    rational, stream_rotation,
};

const PACKET_LIMIT: usize = 512;
const BYTE_LIMIT: usize = 32 * 1024 * 1024;

pub(in crate::video) struct Poster {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub rotation: f64,
}

pub(in crate::video) fn extract_poster(
    path: &Path,
    cancelled: impl Fn() -> bool + Send + Sync,
    maximum: [u32; 2],
) -> Result<Poster> {
    ensure!(
        maximum.iter().all(|&size| (1..=256).contains(&size)),
        "Invalid thumbnail dimensions"
    );
    ensure!(!cancelled(), "Thumbnail request was cancelled");

    let mut input = Input::open(path, &cancelled, Probe::Poster)?;
    let index = input.best_stream(ffi::AVMEDIA_TYPE_VIDEO)?;
    let stream = input.stream(index);
    let mut codec = Codec::software_stream(stream)?;
    let stream_rotation = stream_rotation(stream).unwrap_or(0.0);
    let time_base = rational(stream.time_base);

    // A nearby keyframe avoids decoding a long prefix just to build a poster
    if input.data().duration > 0 && time_base.is_finite() && time_base > 0.0 {
        let seconds = (input.data().duration as f64
            / (10.0 * f64::from(ffi::AV_TIME_BASE)))
        .min(3.0);
        let start = if stream.start_time == i64::MIN {
            0
        } else {
            stream.start_time
        };
        let target = start.saturating_add((seconds / time_base) as i64);

        // SAFETY: The input is exclusively owned; a failed optional seek leaves decoding at the start
        unsafe {
            ffi::av_seek_frame(
                input.raw(),
                index as i32,
                target,
                ffi::AVSEEK_FLAG_BACKWARD as i32,
            )
        };
    }

    let frame = decode(&mut input, index, &mut codec, &cancelled)?;
    ensure!(!cancelled(), "Thumbnail request was cancelled");

    // SAFETY: The retained input, stream and decoded frame remain alive through aspect calculation
    let aspect = unsafe {
        ffi::av_guess_sample_aspect_ratio(
            input.raw(),
            input.stream(index) as *const _ as *mut _,
            frame.raw(),
        )
    };
    let mut poster = scale(&frame, aspect, maximum)?;
    poster.rotation = frame.rotation().unwrap_or(stream_rotation);

    Ok(poster)
}

fn decode(
    input: &mut Input<'_>,
    stream_index: usize,
    codec: &mut Codec,
    cancelled: &impl Fn() -> bool,
) -> Result<DecodedFrame> {
    let mut packets = 0;
    let mut bytes = 0_usize;
    let mut draining = false;

    loop {
        ensure!(!cancelled(), "Thumbnail request was cancelled");

        match codec.receive()? {
            Receive::Item(frame) => return Ok(frame),
            Receive::End => {
                anyhow::bail!("No video frame was decoded for the thumbnail")
            }
            Receive::Again => {}
        }

        ensure!(
            !draining,
            "No delayed video frame was decoded for the thumbnail"
        );
        ensure!(
            packets < PACKET_LIMIT && bytes < BYTE_LIMIT,
            "Thumbnail decoding exceeded its packet budget"
        );

        let Some(packet) = input.read()? else {
            draining = true;
            ensure!(
                !matches!(codec.send(None)?, Sent::Again),
                "Thumbnail decoder did not consume its previous output"
            );
            continue;
        };

        packets += 1;
        bytes = bytes.saturating_add(packet.bytes);
        ensure!(
            bytes <= BYTE_LIMIT,
            "Thumbnail decoding exceeded its byte budget"
        );

        // SAFETY: The input initialized this retained packet's stream index
        if unsafe { (*packet.raw()).stream_index } == stream_index as i32 {
            ensure!(
                !matches!(codec.send(Some(&packet))?, Sent::Again),
                "Thumbnail decoder did not consume its previous output"
            );
        }
    }
}

struct Scaler(NonNull<ffi::SwsContext>);

impl Drop for Scaler {
    fn drop(&mut self) {
        // SAFETY: This wrapper exclusively owns the initialized scaler
        unsafe { ffi::sws_freeContext(self.0.as_ptr()) };
    }
}

fn scaled_size(
    size: [i32; 2],
    aspect: ffi::AVRational,
    maximum: [u32; 2],
) -> Result<[u32; 2]> {
    dimensions(size[0], size[1])?;

    let ratio = if aspect.num > 0 && aspect.den > 0 {
        rational(aspect)
    } else {
        1.0
    };
    let width = f64::from(size[0]) * ratio;
    let height = f64::from(size[1]);
    let factor = 1.0_f64
        .min(f64::from(maximum[0]) / width)
        .min(f64::from(maximum[1]) / height);

    Ok([
        (width * factor).round().clamp(1.0, f64::from(maximum[0])) as u32,
        (height * factor).round().clamp(1.0, f64::from(maximum[1])) as u32,
    ])
}

fn scale(
    frame: &DecodedFrame,
    aspect: ffi::AVRational,
    maximum: [u32; 2],
) -> Result<Poster> {
    let data = frame.data();
    let [width, height] = scaled_size([data.width, data.height], aspect, maximum)?;

    // SAFETY: Dimensions are bounded and the frame has a format returned by FFmpeg
    let scaler = unsafe {
        ffi::sws_getContext(
            data.width,
            data.height,
            data.format,
            width as i32,
            height as i32,
            ffi::AV_PIX_FMT_RGBA,
            ffi::SWS_BILINEAR as i32,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    let scaler =
        Scaler(NonNull::new(scaler).context("Cannot allocate the thumbnail scaler")?);
    let colorspace = match data.colorspace {
        ffi::AVCOL_SPC_BT709 => ffi::SWS_CS_ITU709,
        ffi::AVCOL_SPC_BT2020_NCL | ffi::AVCOL_SPC_BT2020_CL => ffi::SWS_CS_BT2020,
        ffi::AVCOL_SPC_FCC => ffi::SWS_CS_FCC,
        ffi::AVCOL_SPC_SMPTE240M => ffi::SWS_CS_SMPTE240M,
        _ => ffi::SWS_CS_DEFAULT,
    };

    // SAFETY: FFmpeg returns static conversion coefficients and copies the settings
    unsafe {
        let coefficients = ffi::sws_getCoefficients(colorspace as i32);
        check(
            ffi::sws_setColorspaceDetails(
                scaler.0.as_ptr(),
                coefficients,
                i32::from(data.color_range == ffi::AVCOL_RANGE_JPEG),
                coefficients,
                1,
                0,
                65536,
                65536,
            ),
            "Cannot configure thumbnail colorspace",
        )?;
    }

    let mut pixels = vec![0; width as usize * height as usize * 4];
    let planes = [
        pixels.as_mut_ptr(),
        ptr::null_mut(),
        ptr::null_mut(),
        ptr::null_mut(),
    ];
    let strides = [width as i32 * 4, 0, 0, 0];

    // SAFETY: Output storage fits all rows and the source frame retains every input plane
    let rows = check(
        unsafe {
            ffi::sws_scale(
                scaler.0.as_ptr(),
                data.data.as_ptr().cast(),
                data.linesize.as_ptr(),
                0,
                data.height,
                planes.as_ptr(),
                strides.as_ptr(),
            )
        },
        "Cannot scale the video thumbnail",
    )?;
    ensure!(rows == height as i32, "Incomplete video thumbnail");

    Ok(Poster {
        pixels,
        width,
        height,
        rotation: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poster_size_preserves_display_aspect_and_bounds() -> Result<()> {
        assert_eq!(
            scaled_size([320, 180], ffi::AVRational { num: 2, den: 1 }, [192, 128])?,
            [192, 54]
        );
        assert_eq!(
            scaled_size([64, 32], ffi::AVRational { num: 1, den: 1 }, [192, 128])?,
            [64, 32]
        );
        assert_eq!(
            scaled_size([1, 10000], ffi::AVRational { num: 0, den: 0 }, [192, 128])?,
            [1, 128]
        );
        assert!(
            scaled_size([0, 180], ffi::AVRational { num: 1, den: 1 }, [192, 128])
                .is_err()
        );

        Ok(())
    }
}
