//! Small CPU posters with cancellation and a deadline for blocking media I/O

use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use image::DynamicImage;

use super::media;
use crate::services::{
    image_decode::color_image_within, image_workers::THUMBNAIL_SIZE,
};

struct Cancellation<'a> {
    generation: &'a AtomicU64,
    expected: u64,
    deadline: Instant,
}

impl Cancellation<'_> {
    fn cancelled(&self) -> bool {
        self.generation.load(Ordering::Relaxed) != self.expected
            || Instant::now() >= self.deadline
    }
}

pub(super) fn extract(
    path: &Path,
    generation: &AtomicU64,
    expected: u64,
) -> Result<egui::ColorImage> {
    let cancellation = Cancellation {
        generation,
        expected,
        deadline: Instant::now() + Duration::from_secs(3),
    };
    ensure!(!cancellation.cancelled(), "Thumbnail request was cancelled");

    let [max_width, max_height] = THUMBNAIL_SIZE;
    let poster =
        media::extract_poster(path, || cancellation.cancelled(), THUMBNAIL_SIZE)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_cancellation_checks_generation_and_deadline() {
        let generation = AtomicU64::new(4);
        let mut cancellation = Cancellation {
            generation: &generation,
            expected: 4,
            deadline: Instant::now() + Duration::from_secs(3),
        };
        assert!(!cancellation.cancelled());

        generation.store(5, Ordering::Relaxed);
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
        let generation = AtomicU64::new(0);
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

        generation.store(1, Ordering::Relaxed);
        assert!(extract(&clip, &generation, 0).is_err());
        let invalid = folder.path().join("invalid.mp4");
        std::fs::write(&invalid, b"Not a video")?;
        assert!(extract(&invalid, &generation, 1).is_err());
        assert!(extract(&folder.path().join("missing.mp4"), &generation, 1).is_err());

        Ok(())
    }
}
