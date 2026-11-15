//! Unit tests for the video area that need no private access

mod rates {
    use crate::video::{PLAYBACK_RATES, step_rate};

    #[test]
    fn speed_steps_move_between_offered_rates_and_stop_at_the_ends() {
        assert_eq!(step_rate(1., 1), 1.25);
        assert_eq!(step_rate(1., -2), 0.5);
        assert_eq!(step_rate(4., 1), 4.);
        assert_eq!(step_rate(0.25, -1), 0.25);

        // A rate between offered speeds steps from the nearest one
        assert_eq!(step_rate(1.1, 1), 1.25);
        assert!(PLAYBACK_RATES.is_sorted());
    }
}
