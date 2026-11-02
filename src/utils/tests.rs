//! Unit tests for the utils area

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
