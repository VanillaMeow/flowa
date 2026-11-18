//! Playback time from the audio being heard, with a monotonic clock for silent clips
//!
//! Positions are media seconds. At a playback rate other than one, each second of
//! wall time covers `rate` media seconds; the audio feed already accounts for the
//! rate in the position it reports

use std::time::Instant;

pub(super) struct Clock {
    anchor: f64,
    at: Instant,
    pub floor: f64,
    drained_at: Option<Instant>,
    drained_position: f64,
    rate: f64,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            anchor: 0.0,
            at: Instant::now(),
            floor: 0.0,
            drained_at: None,
            drained_position: 0.0,
            rate: 1.0,
        }
    }
}

impl Clock {
    /// The current position, from `heard` audio when there is some, and from wall
    /// time otherwise or once the audio has `drained`
    pub fn position(
        &mut self,
        now: Instant,
        running: bool,
        heard: Option<f64>,
        drained: bool,
    ) -> f64 {
        if !running {
            return self.anchor;
        }

        let position = match heard {
            Some(heard) if !drained => {
                self.drained_at = None;
                heard
            }
            Some(heard) => {
                let drained = *self.drained_at.get_or_insert_with(|| {
                    self.drained_position = heard;
                    now
                });
                self.drained_position + self.elapsed(drained, now)
            }
            None => self.anchor + self.elapsed(self.at, now),
        };

        position.max(self.floor)
    }

    fn elapsed(&self, since: Instant, now: Instant) -> f64 {
        now.saturating_duration_since(since).as_secs_f64() * self.rate
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Changes speed from `position` on, which the caller reads at the old rate
    pub fn set_rate(&mut self, rate: f64, position: f64, now: Instant) {
        self.anchor(position, now);
        self.rate = rate;
    }

    pub fn anchor(&mut self, position: f64, now: Instant) {
        self.anchor = position;
        self.at = now;

        // Reanchoring a paused video tail must exclude time spent paused after audio ended
        if self.drained_at.is_some() {
            self.drained_at = Some(now);
            self.drained_position = position;
        }
    }

    pub fn seek(&mut self, position: f64, floor: f64, now: Instant) {
        self.anchor(position, now);
        self.floor = floor;
        self.drained_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn heard_audio_drives_the_position_while_running() {
        let mut clock = Clock::default();
        assert_eq!(
            clock.position(Instant::now(), true, Some(3.68), false),
            3.68
        );
    }

    #[test]
    fn silent_clock_stays_fixed_while_paused_and_reanchors_on_resume() {
        let now = Instant::now();
        let mut clock = Clock::default();
        clock.anchor(2.0, now);
        assert_eq!(
            clock.position(now + Duration::from_secs(5), false, None, false),
            2.0
        );

        clock.anchor(2.0, now + Duration::from_secs(5));
        assert_eq!(
            clock.position(now + Duration::from_secs(6), true, None, false),
            3.0
        );
    }

    #[test]
    fn drained_audio_allows_the_last_video_frames_to_finish() {
        let now = Instant::now();
        let mut clock = Clock::default();
        assert_eq!(clock.position(now, true, Some(2.0), true), 2.0);
        assert_eq!(
            clock.position(now + Duration::from_secs(1), true, Some(2.0), true),
            3.0
        );
    }

    #[test]
    fn rate_scales_wall_time_and_zero_stops_it() {
        let now = Instant::now();
        let mut silent = Clock::default();
        silent.set_rate(0.5, 4.0, now);
        assert_eq!(
            silent.position(now + Duration::from_secs(2), true, None, false),
            5.0
        );

        silent.set_rate(0.0, 5.0, now + Duration::from_secs(2));
        assert_eq!(
            silent.position(now + Duration::from_secs(9), true, None, false),
            5.0
        );
    }

    #[test]
    fn seeking_clamps_preroll_to_the_floor() {
        let now = Instant::now();
        let mut clock = Clock::default();
        clock.seek(4.0, 4.0, now);
        assert_eq!(clock.position(now, false, None, false), 4.0);
        assert_eq!(clock.position(now, true, Some(3.9), false), 4.0);
    }

    #[test]
    fn pausing_after_audio_drains_does_not_skip_the_remaining_video() {
        let now = Instant::now();
        let mut clock = Clock::default();
        assert_eq!(clock.position(now, true, Some(2.0), true), 2.0);

        let paused = now + Duration::from_secs(1);
        let position = clock.position(paused, true, Some(2.0), true);
        clock.anchor(position, paused);
        assert_eq!(
            clock.position(paused + Duration::from_secs(10), false, Some(2.0), true),
            3.0
        );

        clock.anchor(3.0, paused + Duration::from_secs(10));
        assert_eq!(
            clock.position(paused + Duration::from_secs(11), true, Some(2.0), true),
            4.0
        );
    }
}
