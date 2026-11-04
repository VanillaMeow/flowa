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
