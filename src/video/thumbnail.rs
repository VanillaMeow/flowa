//! Small CPU posters with cancellation and a deadline for blocking media I/O
//!
//! Decodes one frame with a single-threaded software codec and scales it, without
//! audio output or GPU initialization

use std::{
    path::Path,
    ptr,
    ptr::NonNull,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use image::DynamicImage;

use super::{
    decoding::{SoftwareCodec, Threads},
    ffi::ffmpeg as ffi,
    ffmpeg::{
        AvFrame, Input, Probe, Receive, check, dimensions, rational, stream_rotation,
    },
};
use crate::{
    services::{
        image_decode::color_image_within, thumbnails::THUMBNAIL_SIZE,
        worker::Generation,
    },
    utils::units::MIB,
};

const POSTER_PACKET_LIMIT: usize = 512;
const POSTER_BYTE_LIMIT: usize = 32 * MIB as usize;

struct Cancellation<'a> {
    generation: &'a Generation,
    expected: u64,
    deadline: Instant,
}

impl Cancellation<'_> {
    fn cancelled(&self) -> bool {
        !self.generation.is_current(self.expected) || Instant::now() >= self.deadline
    }
}

pub(super) fn extract(
    path: &Path,
    generation: &Generation,
    expected: u64,
) -> Result<egui::ColorImage> {
    let cancellation = Cancellation {
        generation,
        expected,
        deadline: Instant::now() + Duration::from_secs(3),
    };
    ensure!(!cancellation.cancelled(), "Thumbnail request was cancelled");

    let [max_width, max_height] = THUMBNAIL_SIZE;
    let poster = extract_poster(path, || cancellation.cancelled(), THUMBNAIL_SIZE)?;
    ensure!(!cancellation.cancelled(), "Thumbnail request was cancelled");
    ensure!(
        (1..=max_width).contains(&poster.width)
            && (1..=max_height).contains(&poster.height),
        "Invalid video thumbnail dimensions"
    );
    let image = image::RgbaImage::from_raw(poster.width, poster.height, poster.pixels)
        .context("Invalid video thumbnail pixels")?;
    let image = DynamicImage::ImageRgba8(image);
    let quarter_turns = (-poster.rotation / 90.0).round() as i32;
    let image = match quarter_turns.rem_euclid(4) {
        1 => image.rotate90(),
        2 => image.rotate180(),
        3 => image.rotate270(),
        _ => image,
    };

    // Quarter turns can leave a poster taller than the box it was scaled for
    color_image_within(&image.into_rgba8(), THUMBNAIL_SIZE)
}

struct Poster {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    rotation: f64,
}

fn extract_poster(
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
    let mut codec = SoftwareCodec::open(stream, Threads::Single)?;
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

/// Feeds the video stream's packets to the codec until it releases its first frame
fn decode(
    input: &mut Input<'_>,
    stream_index: usize,
    codec: &mut SoftwareCodec,
    cancelled: &impl Fn() -> bool,
) -> Result<AvFrame> {
    let mut packets = 0;
    let mut bytes = 0_usize;

    loop {
        ensure!(!cancelled(), "Thumbnail request was cancelled");
        ensure!(
            packets < POSTER_PACKET_LIMIT && bytes < POSTER_BYTE_LIMIT,
            "Thumbnail decoding exceeded its packet budget"
        );

        let packet = input.read()?;

        if let Some(packet) = &packet {
            packets += 1;
            bytes = bytes.saturating_add(packet.bytes);
            ensure!(
                bytes <= POSTER_BYTE_LIMIT,
                "Thumbnail decoding exceeded its byte budget"
            );

            // SAFETY: The input initialized this retained packet's stream index
            if unsafe { (*packet.raw()).stream_index } != stream_index as i32 {
                continue;
            }
        }

        let mut decoded = None;
        let mut ended = false;

        codec.decode(packet.as_ref(), |output| match output {
            Receive::Item(frame) => {
                decoded = Some(frame);
                false
            }
            Receive::End => {
                ended = true;
                false
            }
            Receive::Again => true,
        })?;

        if let Some(frame) = decoded {
            return Ok(frame.into_frame());
        }

        // Draining at the end of the file releases delayed frames or nothing at all
        ensure!(
            packet.is_some() && !ended,
            "No video frame was decoded for the thumbnail"
        );
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
    frame: &AvFrame,
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

    #[test]
    fn thumbnail_cancellation_checks_generation_and_deadline() {
        let generation = Generation::default();
        generation.set(4);
        let mut cancellation = Cancellation {
            generation: &generation,
            expected: 4,
            deadline: Instant::now() + Duration::from_secs(3),
        };
        assert!(!cancellation.cancelled());

        generation.set(5);
        assert!(cancellation.cancelled());

        cancellation.expected = 5;
        cancellation.deadline = Instant::now();
        assert!(cancellation.cancelled());
    }

    #[test]
    #[ignore = "Requires ffmpeg with libx264"]
    fn video_posters_handle_delayed_frames_aspect_rotation_and_invalid_files()
    -> Result<()> {
        let folder = tempfile::tempdir()?;
        let clip = folder.path().join("poster-ž-日本語.mp4");
        let rotated = folder.path().join("rotated.mp4");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                "color=red:size=320x180:rate=10,drawbox=x=160:y=0:w=160:h=180:color=blue:t=fill,setsar=2/1",
                "-t", "1", "-c:v", "libx264", "-bf", "3",
            ])
            .arg(&clip)
            .status()?;
        ensure!(generated.success(), "Could not generate the poster fixture");
        let generation = Generation::default();
        let started = Instant::now();
        let poster = extract(&clip, &generation, 0)?;
        assert_eq!(poster.size, [192, 54]);
        assert!(poster[(20, 20)].r() > 200);
        assert!(poster[(170, 20)].b() > 200);
        eprintln!(
            "Generated {}×{} video poster in {:?}",
            poster.width(),
            poster.height(),
            started.elapsed()
        );

        let remuxed = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-display_rotation:v:0",
                "90",
                "-i",
            ])
            .arg(&clip)
            .args(["-c", "copy"])
            .arg(&rotated)
            .status()?;
        ensure!(remuxed.success(), "Could not generate the rotated fixture");
        let poster = extract(&rotated, &generation, 0)?;
        assert_eq!(poster.size, [36, 128]);
        assert!(poster[(18, 15)].b() > 200);
        assert!(poster[(18, 110)].r() > 200);

        generation.set(1);
        assert!(extract(&clip, &generation, 0).is_err());
        let invalid = folder.path().join("invalid.mp4");
        std::fs::write(&invalid, b"Not a video")?;
        assert!(extract(&invalid, &generation, 1).is_err());
        assert!(extract(&folder.path().join("missing.mp4"), &generation, 1).is_err());

        Ok(())
    }
}
