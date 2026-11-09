//! Unit tests for the services area

mod folder_scan {
    use anyhow::Result;

    use crate::{services::folder_scan::scan, settings::SortOrder, viewer::catalog};

    #[test]
    fn folder_scan_is_natural_and_ignores_unrelated_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for name in ["image10.PNG", "image2.png", "image1.jpg", "notes.txt"] {
            std::fs::write(dir.path().join(name), b"fixture")?;
        }
        let mut files = scan(dir.path())?;
        catalog::sort(&mut files, SortOrder::Name, false);
        assert_eq!(
            files.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["image1.jpg", "image2.png", "image10.PNG"]
        );
        Ok(())
    }
}

mod image_decode {
    use std::{fs::File, time::Duration};

    use crate::utils::units::MIB;

    use anyhow::Result;
    use image::RgbaImage;

    use crate::{
        services::image_decode::{decode, export, thumbnail},
        viewer::transform::View,
    };

    #[test]
    fn static_decode_and_export_preserve_alpha_and_apply_rotation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("source.png");
        let output = dir.path().join("export.png");
        let pixels = RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([x as u8 * 50, y as u8 * 80, 90, 40 + (x + y * 3) as u8 * 30])
        });
        pixels.save(&input)?;
        let original = std::fs::read(&input)?;
        let loaded = decode(&input, 32 * MIB)?;
        assert_eq!(loaded.frames[0].pixels.as_ref(), &pixels);

        // Source rows are ABC / DEF; a clockwise turn produces DA / EB / FC
        for (flip_x, flip_y, expected) in [
            (false, false, [3, 0, 4, 1, 5, 2]),
            (true, false, [5, 2, 4, 1, 3, 0]),
            (false, true, [0, 3, 1, 4, 2, 5]),
        ] {
            let view = View {
                rotation: 1,
                flip_x,
                flip_y,
                ..View::default()
            };
            export(&loaded.frames[0].pixels, &output, &view)?;
            let result = image::open(&output)?.into_rgba8();
            assert_eq!(result.dimensions(), (2, 3));
            let expected: Vec<_> = expected
                .into_iter()
                .map(|index| *pixels.get_pixel(index % 3, index / 3))
                .collect();
            assert_eq!(result.pixels().copied().collect::<Vec<_>>(), expected);
        }

        assert_eq!(std::fs::read(input)?, original);
        assert_eq!(loaded.frames[0].pixels.as_ref(), &pixels);

        Ok(())
    }

    #[test]
    fn corrupt_images_fail_without_panicking() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("bad.png");
        std::fs::write(&path, b"not an image")?;
        assert!(decode(&path, 32 * MIB).is_err());
        Ok(())
    }

    #[test]
    fn decode_respects_memory_budget_and_sizes_previews() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("wide.png");
        RgbaImage::new(640, 320).save(&path)?;

        let error = decode(&path, 1024)
            .err()
            .expect("Decode exceeded its budget");
        assert!(format!("{error:#}").to_ascii_lowercase().contains("limit"));

        let loaded = decode(&path, 32 * MIB)?;
        assert_eq!(loaded.frames[0].preview.size, [640, 320]);
        assert_eq!(loaded.thumbnail.size, [192, 96]);

        Ok(())
    }

    #[test]
    fn svg_alpha_is_unpremultiplied() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("transparent.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000" opacity="0.5"/></svg>"##,
        )?;
        let decoded = decode(&path, 32 * MIB)?;
        let px = decoded.frames[0].pixels.get_pixel(5, 5);
        assert_eq!(px[0], 255);
        assert!((126..=129).contains(&px[3]));
        Ok(())
    }

    const SPLIT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="10" height="10" fill="#ff0000"/><rect x="10" width="10" height="10" fill="#0000ff"/></svg>"##;

    #[test]
    fn small_svg_keeps_its_size_with_an_enlarged_preview() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("small.svg");
        std::fs::write(&path, SPLIT_SVG)?;

        let decoded = decode(&path, 32 * MIB)?;
        assert_eq!(decoded.size, [20, 10]);
        assert!(decoded.vector.is_some());
        assert_eq!(decoded.frames[0].preview.size, [2048, 1024]);
        assert_eq!(decoded.thumbnail.size, [192, 96]);
        assert_eq!(thumbnail(&path, 32 * MIB)?.size, [192, 96]);

        Ok(())
    }

    #[test]
    fn vector_rendering_stays_sharp_when_enlarged() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("split.svg");
        std::fs::write(&path, SPLIT_SVG)?;
        let decoded = decode(&path, 32 * MIB)?;
        let vector = decoded
            .vector
            .expect("SVG decodes keep their vector source");

        // Sixty-four samples per image pixel around the color boundary at x = 10
        let pixels = vector.render(egui::pos2(9.5, 0.0), 64.0, [64, 8])?;
        assert_eq!(pixels.get_pixel(31, 4).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(32, 4).0, [0, 0, 255, 255]);

        Ok(())
    }

    #[test]
    fn gif_animation_retains_frame_delays() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("animated.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&path)?);
        for c in [40, 200] {
            encoder.encode_frame(image::Frame::from_parts(
                RgbaImage::from_pixel(8, 8, image::Rgba([c, 0, 0, 255])),
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            ))?;
        }
        drop(encoder);
        let decoded = decode(&path, MIB)?;
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.frames[0].delay, Duration::from_millis(100));
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod system_trash {
    use crate::services::system_trash::delete;
    use anyhow::Result;

    #[test]
    fn trashing_a_symlink_does_not_trash_its_target() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join("original.png");
        let link = directory.path().join("link.png");
        std::fs::write(&target, b"Keep the original image")?;
        std::os::unix::fs::symlink(&target, &link)?;

        let error = delete(&link)
            .expect_err("A symlink must be rejected before connecting to D-Bus");
        assert!(error.to_string().contains("Could not open"));
        assert!(link.is_symlink());
        assert_eq!(std::fs::read(target)?, b"Keep the original image");

        Ok(())
    }
}

mod resize {
    use anyhow::Result;
    use image::{Rgba, RgbaImage};

    use crate::services::resize::{Region, downscale};

    #[test]
    fn transparent_pixels_do_not_tint_their_neighbors() -> Result<()> {
        let mut pixels = RgbaImage::new(2, 1);
        pixels.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        pixels.put_pixel(1, 0, Rgba([0, 255, 0, 0]));

        let averaged = downscale(&pixels, None, [1, 1])?;
        let [red, green, _, alpha] = averaged.get_pixel(0, 0).0;

        assert!(red > 245, "Red was {red}");
        assert!(
            green < 10,
            "A transparent green pixel tinted the result: {green}"
        );
        assert!((120..=135).contains(&alpha));
        Ok(())
    }

    #[test]
    fn regions_select_source_pixels() -> Result<()> {
        let pixels = RgbaImage::from_fn(4, 2, |x, _| {
            Rgba([if x < 2 { 0 } else { 200 }, 0, 0, 255])
        });
        let region = Region {
            x: 2,
            y: 0,
            width: 2,
            height: 2,
        };

        let tile = downscale(&pixels, Some(region), [1, 1])?;

        assert_eq!(tile.get_pixel(0, 0).0, [200, 0, 0, 255]);
        Ok(())
    }
}

mod worker {
    use std::time::Duration;

    use crate::services::worker::{Generation, Tagged, Worker};

    struct Job(u64);

    impl Tagged for Job {
        fn generation(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn tagged_workers_skip_requests_from_other_generations() {
        let generation = Generation::default();
        generation.set(2);
        let worker = Worker::spawn_tagged(
            "test",
            egui::Context::default(),
            &generation,
            |job: Job| Some(job.0),
        );

        assert!(worker.request(Job(1)));
        assert!(worker.request(Job(2)));

        let received = worker.results.recv_timeout(Duration::from_secs(3));
        assert_eq!(received.ok(), Some(2));
        assert!(
            worker
                .results
                .recv_timeout(Duration::from_millis(50))
                .is_err()
        );
    }
}

mod tiles {
    use std::sync::Arc;

    use crate::services::tiles::{TileSource, tile_level, tile_span};

    #[test]
    fn tile_levels_supply_at_least_the_requested_density() {
        assert_eq!(tile_level(1.0), 0);
        assert_eq!(tile_level(0.3), 1);
        assert_eq!(tile_level(40.0), -6);
        assert_eq!(tile_level(100_000.0), -9);
        assert_eq!(tile_span(-3), 128.0);
    }

    #[test]
    fn raster_tiles_stop_at_one_sample_per_pixel() {
        let raster = TileSource::Raster(Arc::new(image::RgbaImage::new(1, 1)));

        assert_eq!(raster.max_density(), 1.0);
    }
}
