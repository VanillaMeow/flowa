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

mod subtitles {
    use crate::video::{
        Track, Tracks,
        subtitles::{
            Cue, CueContent, Span, SubtitleCues, SubtitleText, parse_ass_event,
            parse_ass_text,
        },
    };

    fn text_cue(start: f64, end: f64, text: &str) -> Cue {
        Cue {
            start,
            end,
            content: CueContent::Text(SubtitleText {
                spans: vec![Span {
                    text: text.into(),
                    italic: false,
                }],
                top: false,
            }),
        }
    }

    fn shown(cues: &SubtitleCues, position: f64) -> Vec<String> {
        cues.active(position)
            .iter()
            .filter_map(|cue| match &cue.content {
                CueContent::Text(text) => Some(text.spans[0].text.clone()),
                CueContent::Bitmap(_) => None,
            })
            .collect()
    }

    #[test]
    fn ass_events_keep_text_line_breaks_and_italics() {
        let parsed =
            parse_ass_event("0,0,Default,,0,0,0,,Hello, {\\i1}there{\\i0}\\Nfriend")
                .expect("A cue");

        assert!(!parsed.top);
        assert_eq!(
            parsed.spans,
            vec![
                Span {
                    text: "Hello, ".into(),
                    italic: false
                },
                Span {
                    text: "there".into(),
                    italic: true
                },
                Span {
                    text: "\nfriend".into(),
                    italic: false
                },
            ]
        );
    }

    #[test]
    fn top_alignment_is_kept_and_drawings_and_tags_are_dropped() {
        let parsed = parse_ass_text("{\\an8\\pos(10,20)}Sign{\\p1}m 0 0 l 10 10{\\p0}")
            .expect("A cue");

        assert!(parsed.top);
        assert_eq!(parsed.spans.len(), 1);
        assert_eq!(parsed.spans[0].text, "Sign");
        assert!(parse_ass_text("{\\p1}m 0 0 l 1 1{\\p0}").is_none());
        assert!(
            parse_ass_text("{\\iclip(0,0,5,5)}Clip")
                .is_some_and(|text| !text.spans[0].italic)
        );
    }

    #[test]
    fn cues_show_in_their_interval_and_open_ones_last_until_the_next() {
        let mut cues = SubtitleCues::default();
        cues.insert(text_cue(4., 6., "second"));
        cues.insert(text_cue(1., 3., "first"));
        cues.insert(text_cue(1., 3., "first"));
        cues.insert(text_cue(7., f64::INFINITY, "open"));
        cues.insert(text_cue(9., 10., "after"));

        assert_eq!(
            cues.len(),
            4,
            "A cue read again after a seek was kept twice"
        );
        assert_eq!(shown(&cues, 2.), ["first"]);
        assert!(shown(&cues, 3.5).is_empty());
        assert_eq!(shown(&cues, 8.9), ["open"]);
        assert_eq!(shown(&cues, 9.5), ["after"]);

        cues.clear_at(8.);
        assert!(shown(&cues, 8.5).is_empty());
    }

    #[test]
    fn preferred_subtitles_are_forced_then_default_then_files() {
        let track = |index, default, forced, external| Track {
            index,
            label: String::new(),
            default,
            forced,
            external,
        };
        let mut tracks = Tracks {
            audio: Vec::new(),
            subtitles: vec![
                track(2, false, false, false),
                track(9, false, false, true),
            ],
        };
        assert_eq!(tracks.preferred_subtitles(), Some(9));

        tracks.subtitles.push(track(3, true, false, false));
        assert_eq!(tracks.preferred_subtitles(), Some(3));

        tracks.subtitles.push(track(4, false, true, false));
        assert_eq!(tracks.preferred_subtitles(), Some(4));

        tracks.subtitles.clear();
        assert_eq!(tracks.preferred_subtitles(), None);
    }
}
