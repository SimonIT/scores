//! RTTTL (Ring Tone Text Transfer Language) plugin, on top of the [`rtttl`]
//! crate for the actual grammar (note letters, sharps, dot/double-dot and
//! the triplet `&` modifier, default-parameter inheritance).
//!
//! RTTTL is a monophonic, bar-line-free format with no key/time signature
//! and no metadata beyond a  short name, so reading always produces a
//! single [`scores_core::Part`] with a single [`scores_core::Measure`]
//! holding every event, and writing only ever uses the *first* part of the
//! score (any others are dropped) and only the first note of a chord.
//! `rtttl` only supports reading, so writing is hand-rolled here.
//!
//! RTTTL has no syntax for dynamics, articulations, ornaments, fermatas,
//! chord symbols, barlines, repeats/endings or navigation marks, so none of
//! those are ever produced on read. On write, repeats, endings and
//! D.C./D.S./Coda/Fine jumps are unrolled into playback order (a ringtone is
//! a performance), and everything else is dropped - grace notes included,
//! since RTTTL has no way to write a tone that takes no time.
//!
//! `rtttl::parse` resolves each tone straight to `(frequency, wall-clock
//! duration)`, discarding the note letter/octave and the beats-per-minute
//! used to compute that duration; both are recovered here (frequency via
//! the standard equal-tempered formula, BPM by re-reading the `b=` default
//! parameter directly, since the crate never exposes it).

mod read;
mod write;

use scores_core::{Format, FormatReader, FormatWriter, Result, Score};

pub struct RtttlFormat;

impl FormatReader for RtttlFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for RtttlFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for RtttlFormat {
    fn id(&self) -> &'static str {
        "rtttl"
    }

    fn name(&self) -> &'static str {
        "RTTTL"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rtttl", "txt"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::{Duration, Event, Measure, Note, NoteValue, Part, Pitch, Score, Step, Tie};

    const FIFTH: &str = "fifth:d=4,o=5,b=63:8p,8g5,8g5,8g5,2d#5";

    #[test]
    fn reads_a_simple_ringtone() {
        let score = RtttlFormat.read(FIFTH.as_bytes()).unwrap();
        assert_eq!(score.title.as_deref(), Some("fifth"));
        assert_eq!(score.parts.len(), 1);

        let events = &score.parts[0].measures[0].voices[0].events;
        assert_eq!(events.len(), 5);
        let Event::Rest(rest) = &events[0] else {
            panic!("expected a rest")
        };
        assert_eq!(rest.duration.base, NoteValue::Eighth);

        let Event::Note(note) = &events[1] else {
            panic!("expected a note")
        };
        assert_eq!(note.pitch.step, Step::G);
        assert_eq!(note.pitch.octave, 6);

        let Event::Note(last) = &events[4] else {
            panic!("expected a note")
        };
        assert_eq!(last.duration.base, NoteValue::Half);
    }

    #[test]
    fn round_trips_through_write_and_read() {
        let original = RtttlFormat.read(FIFTH.as_bytes()).unwrap();
        let written = RtttlFormat.write(&original).unwrap();
        let reread = RtttlFormat.read(&written).unwrap();

        let a = &original.parts[0].measures[0].voices[0].events;
        let b = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            match (x, y) {
                (Event::Note(x), Event::Note(y)) => {
                    assert_eq!(x.pitch, y.pitch);
                    assert_eq!(x.duration.quarter_length(), y.duration.quarter_length());
                }
                (Event::Rest(x), Event::Rest(y)) => {
                    assert_eq!(x.duration.quarter_length(), y.duration.quarter_length())
                }
                _ => panic!("event kind mismatch"),
            }
        }
    }

    #[test]
    fn flat_pitches_are_respelled_as_sharps_when_writing() {
        let score = Score {
            title: Some("flats".to_string()),
            composer: None,
            parts: vec![Part {
                id: "P1".to_string(),
                name: None,
                measures: vec![Measure::single_voice(
                    1,
                    Default::default(),
                    vec![Event::Note(Note::new(
                        Pitch::new(Step::E, -1, 5),
                        Duration::new(NoteValue::Quarter),
                        Tie::None,
                    ))],
                )],
            }],
        };

        let bytes = RtttlFormat.write(&score).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("4d#4"), "expected a respelled D# in {text}");
    }

    #[test]
    fn grace_notes_are_dropped_when_writing() {
        let mut grace = Note::new(
            Pitch::new(Step::D, 0, 5),
            Duration::new(NoteValue::Eighth),
            Tie::None,
        );
        grace.grace = true;
        let main = Note::new(
            Pitch::new(Step::C, 0, 5),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        );
        let score = Score {
            title: Some("grace".to_string()),
            composer: None,
            parts: vec![Part {
                id: "P1".to_string(),
                name: None,
                measures: vec![Measure::single_voice(
                    1,
                    Default::default(),
                    vec![Event::Note(grace), Event::Note(main)],
                )],
            }],
        };

        let text = String::from_utf8(RtttlFormat.write(&score).unwrap()).unwrap();
        assert!(
            text.ends_with(":4c4"),
            "expected only the main note in {text}"
        );
    }

    #[test]
    fn repeats_are_unrolled_when_writing() {
        let note = |step| {
            Event::Note(Note::new(
                Pitch::new(step, 0, 5),
                Duration::new(NoteValue::Quarter),
                Tie::None,
            ))
        };
        let mut first = Measure::single_voice(1, Default::default(), vec![note(Step::C)]);
        first.barline.repeat_end = Some(2);
        let second = Measure::single_voice(2, Default::default(), vec![note(Step::D)]);
        let score = Score {
            title: Some("repeat".to_string()),
            composer: None,
            parts: vec![Part {
                id: "P1".to_string(),
                name: None,
                measures: vec![first, second],
            }],
        };

        let text = String::from_utf8(RtttlFormat.write(&score).unwrap()).unwrap();
        assert!(text.ends_with(":4c4,4c4,4d4"), "expected C C D in {text}");
    }
}
