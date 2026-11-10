//! Playback time derived from queued audio, with a monotonic clock for silent clips

use std::time::Instant;

#[derive(Clone, Copy)]
pub(super) struct AudioTiming {
    pub queued: f64,
    pub latency: f64,
}

pub(super) struct Clock {
    anchor: f64,
    at: Instant,
    pub floor: f64,
    pub audio_end: Option<f64>,
    drained_at: Option<Instant>,
    drained_position: f64,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            anchor: 0.0,
            at: Instant::now(),
            floor: 0.0,
            audio_end: None,
            drained_at: None,
            drained_position: 0.0,
        }
    }
}

impl Clock {
    pub fn position(
        &mut self,
        now: Instant,
        running: bool,
        audio: Option<AudioTiming>,
        eof: bool,
    ) -> f64 {
        if !running {
            return self.anchor;
        }

        let position = if let (Some(end), Some(audio)) = (self.audio_end, audio) {
            let mut position = end - audio.queued - audio.latency;
            if audio.queued > 0.0 {
                self.drained_at = None;
            } else if eof {
                let drained = match self.drained_at {
                    Some(drained) => drained,
                    None => {
                        self.drained_at = Some(now);
                        self.drained_position = position;
                        now
                    }
                };
                position = self.drained_position
                    + now.saturating_duration_since(drained).as_secs_f64();
            }
            position
        } else {
            self.anchor + now.saturating_duration_since(self.at).as_secs_f64()
        };
        position.max(self.floor)
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
        self.audio_end = None;
        self.drained_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn audio_clock_subtracts_buffered_samples_and_device_latency() {
        let mut clock = Clock {
            audio_end: Some(4.0),
            ..Clock::default()
        };
        let position = clock.position(
            Instant::now(),
            true,
            Some(AudioTiming {
                queued: 0.3,
                latency: 0.02,
            }),
            false,
        );
        assert!((position - 3.68).abs() < 1e-9);
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
        let mut clock = Clock {
            audio_end: Some(2.0),
            ..Clock::default()
        };
        let drained = Some(AudioTiming {
            queued: 0.0,
            latency: 0.0,
        });
        assert_eq!(clock.position(now, true, drained, true), 2.0);
        assert_eq!(
            clock.position(now + Duration::from_secs(1), true, drained, true),
            3.0
        );
    }

    #[test]
    fn seek_discards_the_old_audio_clock_and_clamps_preroll() {
        let now = Instant::now();
        let mut clock = Clock {
            audio_end: Some(30.0),
            ..Clock::default()
        };
        clock.seek(4.0, 4.0, now);
        assert!(clock.audio_end.is_none());
        assert_eq!(clock.position(now, false, None, false), 4.0);

        clock.audio_end = Some(3.9);
        assert_eq!(
            clock.position(
                now,
                true,
                Some(AudioTiming {
                    queued: 0.2,
                    latency: 0.02
                }),
                false
            ),
            4.0
        );
    }

    #[test]
    fn pausing_after_audio_drains_does_not_skip_the_remaining_video() {
        let now = Instant::now();
        let mut clock = Clock {
            audio_end: Some(2.0),
            ..Clock::default()
        };
        let drained = Some(AudioTiming {
            queued: 0.0,
            latency: 0.0,
        });
        assert_eq!(clock.position(now, true, drained, true), 2.0);

        let paused = now + Duration::from_secs(1);
        let position = clock.position(paused, true, drained, true);
        clock.anchor(position, paused);
        assert_eq!(
            clock.position(paused + Duration::from_secs(10), false, drained, true),
            3.0
        );

        clock.anchor(3.0, paused + Duration::from_secs(10));
        assert_eq!(
            clock.position(paused + Duration::from_secs(11), true, drained, true),
            4.0
        );
    }
}
