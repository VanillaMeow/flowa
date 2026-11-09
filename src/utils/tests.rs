//! Unit tests for the utils area

mod format {
    use crate::utils::format::media_time;

    #[test]
    fn playback_times_preserve_hours_and_ignore_invalid_input() {
        for (seconds, expected) in [
            (0., "0:00"),
            (59.9, "0:59"),
            (60., "1:00"),
            (3599., "59:59"),
            (3600., "1:00:00"),
            (3675., "1:01:15"),
            (-1., "0:00"),
            (f64::NAN, "0:00"),
            (f64::INFINITY, "0:00"),
        ] {
            assert_eq!(media_time(seconds), expected);
        }
    }
}

mod file_size {
    use crate::utils::{
        format::file_size,
        units::{GIB, KIB, MIB},
    };

    #[test]
    fn sizes_use_binary_units() {
        for (bytes, expected) in [
            (0, "0 B"),
            (1023, "1023 B"),
            (KIB, "1.0 KiB"),
            (3 * MIB / 2, "1.5 MiB"),
            (3 * GIB, "3.00 GiB"),
        ] {
            assert_eq!(file_size(bytes), expected);
        }
    }
}

mod filename {
    use crate::utils::filename::is_valid;

    #[test]
    fn filename_validation_prevents_traversal_and_windows_reserved_names() {
        for s in [
            "../a.png", "a/b.png", "CON.png", "..", "", "a.png.", "LPT1.jpg",
        ] {
            assert!(!is_valid(s), "accepted {s}");
        }
        assert!(is_valid("summer photo 01.png"));
    }
}

mod size {
    use crate::utils::size::{rgba_bytes, scale_to_fit, shrink_to_fit};

    #[test]
    fn shrinking_keeps_aspect_ratio_and_never_enlarges() {
        assert_eq!(shrink_to_fit([6000, 4000], [2048, 2048]), [2048, 1365]);
        assert_eq!(shrink_to_fit([640, 320], [192, 192]), [192, 96]);
        assert_eq!(shrink_to_fit([100, 50], [192, 128]), [100, 50]);
        assert_eq!(shrink_to_fit([1, 10_000], [192, 128]), [1, 128]);
    }

    #[test]
    fn scaling_enlarges_small_sizes_to_fill_one_axis() {
        assert_eq!(scale_to_fit([20, 10], [2048, 2048]), [2048, 1024]);
        assert_eq!(scale_to_fit([24, 24], [192, 128]), [128, 128]);
        assert_eq!(scale_to_fit([6000, 4000], [2048, 2048]), [2048, 1365]);
    }

    #[test]
    fn rgba_sizes_count_four_bytes_per_pixel() {
        assert_eq!(rgba_bytes([100_000, 100_000]), 40_000_000_000);
    }
}
