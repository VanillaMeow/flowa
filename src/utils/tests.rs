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
    use crate::utils::format::file_size;

    #[test]
    fn sizes_use_binary_units() {
        for (bytes, expected) in [
            (0, "0 B"),
            (1023, "1023 B"),
            (1024, "1.0 KiB"),
            (1536 * 1024, "1.5 MiB"),
            (3 * 1024 * 1024 * 1024, "3.00 GiB"),
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
