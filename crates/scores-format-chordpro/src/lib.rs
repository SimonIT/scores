//! ChordPro plugin, on top of the [`chordpro`] crate for parsing the
//! chords-and-lyrics text format (`chordpro` has no writer for a whole
//! [`Song`], so writing is hand-rolled here). The crate's own chord parser
//! is too narrow for real-world chord names, so the reader only uses it for
//! the song structure and parses each chord's original text itself, with
//! [`scores_core::ChordSymbol::parse`].
//!
//! ChordPro is fundamentally a different kind of thing than a notated
//! score - chord symbols placed over lines of lyrics, with no rhythm and no
//! multi-instrument concept - so this mapping is inherently lossy in both
//! directions. See the [`read`] and [`write`] module docs for exactly how.
//! In short:
//!
//! - **Chord symbols** are the native content: every read event carries
//!   its [`ChordSymbol`](scores_core::ChordSymbol) (and is also voiced as
//!   notes), and the writer emits a score's chord symbols when it has any,
//!   falling back to naming each event's pitch set otherwise.
//! - **Navigation marks** (segno, coda, D.C./D.S. variants, Fine, To Coda)
//!   are written/read as standard `{comment: D.S. al Coda}` directives.
//! - **Dynamics and fermatas** are carried through via a non-standard
//!   bolt-on `<token>` text convention (see [`convert`]).
//! - Multi-voice, hairpins, articulations, ornaments, grace notes, repeats,
//!   volta endings and barline styles have no equivalent in ChordPro's
//!   chord/lyric lines and are not attempted.
//!
//! [`Song`]: chordpro::Song

mod convert;
mod read;
mod write;

use scores_core::{Format, FormatReader, FormatWriter, Result, Score};

pub struct ChordProFormat;

impl FormatReader for ChordProFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for ChordProFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for ChordProFormat {
    fn id(&self) -> &'static str {
        "chordpro"
    }

    fn name(&self) -> &'static str {
        "ChordPro"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["cho", "crd", "chopro", "chordpro"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::Event;

    const SONG: &str = "{title: Test Song}\n{artist: Someone}\n\nThis is [C]a line with a [G]chord\nAnother [Am]line [F]here\n";

    #[test]
    fn reads_chords_from_lyric_lines() {
        let score = ChordProFormat.read(SONG.as_bytes()).unwrap();
        assert_eq!(score.title.as_deref(), Some("Test Song"));
        assert_eq!(score.composer.as_deref(), Some("Someone"));
        assert_eq!(score.parts.len(), 1);
        assert_eq!(score.parts[0].measures.len(), 2);
        assert_eq!(score.parts[0].measures[0].voices[0].events.len(), 2);
    }

    #[test]
    fn round_trips_recognizable_chords() {
        let original = ChordProFormat.read(SONG.as_bytes()).unwrap();
        let written = ChordProFormat.write(&original).unwrap();
        let reread = ChordProFormat.read(&written).unwrap();

        assert_eq!(
            reread.parts[0].measures.len(),
            original.parts[0].measures.len()
        );
        for (a, b) in original.parts[0]
            .measures
            .iter()
            .zip(&reread.parts[0].measures)
        {
            let a_events = &a.voices[0].events;
            let b_events = &b.voices[0].events;
            assert_eq!(a_events.len(), b_events.len());
            for (ea, eb) in a_events.iter().zip(b_events) {
                let pitches = |e: &Event| match e {
                    Event::Note(n) => vec![n.pitch.to_midi_number() % 12],
                    Event::Chord(notes) => notes
                        .iter()
                        .map(|n| n.pitch.to_midi_number() % 12)
                        .collect(),
                    Event::Rest(_) => vec![],
                };
                assert_eq!(pitches(ea), pitches(eb));
            }
        }
    }

    #[test]
    fn dynamics_and_fermata_round_trip_through_bolt_on_markers() {
        let song = "{title: Dyn}\n\n<mf>[C] <ferm>[G]\n";
        let score = ChordProFormat.read(song.as_bytes()).unwrap();
        let events = &score.parts[0].measures[0].voices[0].events;

        let dynamic_of = |e: &Event| match e {
            Event::Note(n) => n.dynamic.clone(),
            Event::Chord(notes) => notes.first().and_then(|n| n.dynamic.clone()),
            Event::Rest(_) => None,
        };
        let fermata_of = |e: &Event| match e {
            Event::Note(n) => n.fermata,
            Event::Chord(notes) => notes.first().is_some_and(|n| n.fermata),
            Event::Rest(_) => false,
        };

        assert_eq!(dynamic_of(&events[0]), Some(scores_core::Dynamic::Mf));
        assert!(fermata_of(&events[1]));

        let written = ChordProFormat.write(&score).unwrap();
        let reread = ChordProFormat.read(&written).unwrap();
        let reread_events = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(
            dynamic_of(&reread_events[0]),
            Some(scores_core::Dynamic::Mf)
        );
        assert!(fermata_of(&reread_events[1]));
    }

    fn symbols(score: &Score) -> Vec<Option<String>> {
        score.parts[0]
            .measures
            .iter()
            .flat_map(|m| &m.voices[0].events)
            .map(|e| e.chord_symbol().map(ToString::to_string))
            .collect()
    }

    fn pitch_classes(event: &Event) -> Vec<i32> {
        let mut classes: Vec<i32> = match event {
            Event::Note(n) => vec![n.pitch.to_midi_number() % 12],
            Event::Chord(notes) => notes
                .iter()
                .map(|n| n.pitch.to_midi_number() % 12)
                .collect(),
            Event::Rest(_) => vec![],
        };
        classes.sort_unstable();
        classes
    }

    #[test]
    fn reads_chord_symbols_verbatim_and_voices_them() {
        // None of these but `Ebmaj7/G` survive the `chordpro` crate's own
        // chord parser intact (and several make it reject the whole song).
        let song = "[Ebmaj7/G]a [F#m7b5]b [C13]c [N.C.]d [G7b9]e [Cmadd9]f\n";
        let score = ChordProFormat.read(song.as_bytes()).unwrap();
        assert_eq!(
            symbols(&score),
            ["Ebmaj7/G", "F#m7b5", "C13", "N.C.", "G7b9", "Cmadd9"].map(|s| Some(s.to_string()))
        );

        let events = &score.parts[0].measures[0].voices[0].events;
        // Slash bass G below Eb-G-Bb-D.
        assert_eq!(pitch_classes(&events[0]), vec![2, 3, 7, 7, 10]);
        assert_eq!(pitch_classes(&events[1]), vec![0, 4, 6, 9]);
        assert!(matches!(events[3], Event::Rest(_)));
        // Unknown suffixes are voiced as their leading quality: G7, Cm.
        assert_eq!(pitch_classes(&events[4]), vec![2, 5, 7, 11]);
        assert_eq!(pitch_classes(&events[5]), vec![0, 3, 7]);
        // A flat root is spelled with flats.
        let Event::Chord(notes) = &events[0] else {
            panic!("expected a chord");
        };
        assert_eq!(notes[1].pitch.alter, -1);
    }

    #[test]
    fn chord_symbols_round_trip_as_written() {
        let song = "[Ebmaj7/G]a [F#m7b5] <ferm>[N.C.]c [Dsus4]d\n";
        let score = ChordProFormat.read(song.as_bytes()).unwrap();
        let written = String::from_utf8(ChordProFormat.write(&score).unwrap()).unwrap();
        assert!(written.contains("[Ebmaj7/G] [F#m7b5] <ferm>[N.C.] [Dsus4]"));
        let reread = ChordProFormat.read(written.as_bytes()).unwrap();
        assert_eq!(symbols(&reread), symbols(&score));
        // A fermata over "N.C." lands on the rest it becomes.
        let events = &reread.parts[0].measures[0].voices[0].events;
        assert!(matches!(&events[2], Event::Rest(rest) if rest.fermata));
    }

    #[test]
    fn ignores_non_chord_brackets_and_directive_chords() {
        let song = "# a [comment] line\n{c: [D] intro}\n{soc}[C]one [*Rit.]two [G]three\n{eoc}\n";
        let score = ChordProFormat.read(song.as_bytes()).unwrap();
        assert_eq!(
            symbols(&score),
            vec![Some("C".to_string()), Some("G".to_string())]
        );
    }

    #[test]
    fn lead_sheet_writes_chord_symbols_not_melody_notes() {
        use scores_core::{
            ChordSymbol, Duration, Measure, MeasureAttributes, Note, NoteValue, Part, Pitch, Rest,
            Step, Tie,
        };
        let note = |step, symbol: Option<&str>| {
            let mut note = Note::new(
                Pitch::new(step, 0, 5),
                Duration::new(NoteValue::Quarter),
                Tie::None,
            );
            note.chord_symbol = symbol.and_then(ChordSymbol::parse);
            Event::Note(note)
        };
        let mut rest = Rest::new(Duration::new(NoteValue::Whole));
        rest.chord_symbol = ChordSymbol::parse("Am7");
        let measures = vec![
            vec![
                note(Step::E, Some("C")),
                note(Step::D, None),
                note(Step::B, Some("G/B")),
                note(Step::C, None),
            ],
            vec![note(Step::E, None); 4],
            vec![Event::Rest(rest)],
        ]
        .into_iter()
        .enumerate()
        .map(|(i, events)| {
            Measure::single_voice(i as u32 + 1, MeasureAttributes::default(), events)
        })
        .collect();
        let score = Score {
            title: None,
            composer: None,
            parts: vec![Part {
                id: "P1".into(),
                name: None,
                measures,
            }],
        };

        let written = String::from_utf8(ChordProFormat.write(&score).unwrap()).unwrap();
        assert_eq!(written, "\n[C] [G/B] \n[Am7] \n");
    }

    #[test]
    fn symbol_less_chords_are_recognized_from_pitches() {
        use scores_core::{
            Duration, Measure, MeasureAttributes, Note, NoteValue, Part, Pitch, Step, Tie,
        };
        let chord = |pitches: &[(Step, i8, i8)]| {
            Event::Chord(
                pitches
                    .iter()
                    .map(|&(step, alter, octave)| {
                        Note::new(
                            Pitch::new(step, alter, octave),
                            Duration::new(NoteValue::Quarter),
                            Tie::None,
                        )
                    })
                    .collect(),
            )
        };
        let events = vec![
            // Root position A minor 7 - not `C6/A`.
            chord(&[
                (Step::A, 0, 3),
                (Step::C, 0, 4),
                (Step::E, 0, 4),
                (Step::G, 0, 4),
            ]),
            chord(&[(Step::B, -1, 3), (Step::D, 0, 4), (Step::F, 0, 4)]),
            chord(&[(Step::G, 0, 3), (Step::C, 0, 4), (Step::E, 0, 4)]),
        ];
        let score = Score {
            title: None,
            composer: None,
            parts: vec![Part {
                id: "P1".into(),
                name: None,
                measures: vec![Measure::single_voice(
                    1,
                    MeasureAttributes::default(),
                    events,
                )],
            }],
        };
        let written = String::from_utf8(ChordProFormat.write(&score).unwrap()).unwrap();
        assert_eq!(written, "\n[Am7] [Bb] [C/G] \n");
    }

    #[test]
    fn navigation_round_trips_through_comments() {
        use scores_core::Navigation;
        let song = "{c: Segno}\n[C]a\n{comment: Play twice}\n[G]b\n{comment: D.S. al Coda}\n{c: coda}\n[F]c\n{c: Fine}\n";
        let score = ChordProFormat.read(song.as_bytes()).unwrap();
        let navigation = |score: &Score| -> Vec<_> {
            score.parts[0]
                .measures
                .iter()
                .map(|m| m.navigation.clone())
                .collect()
        };
        assert_eq!(
            navigation(&score),
            vec![
                vec![Navigation::Segno],
                vec![Navigation::DalSegnoAlCoda],
                vec![Navigation::Coda, Navigation::Fine],
            ]
        );

        let written = String::from_utf8(ChordProFormat.write(&score).unwrap()).unwrap();
        assert_eq!(
            written,
            "\n{comment: Segno}\n[C] \n[G] \n{comment: D.S. al Coda}\n{comment: Coda}\n[F] \n{comment: Fine}\n"
        );
        let reread = ChordProFormat.read(written.as_bytes()).unwrap();
        assert_eq!(navigation(&reread), navigation(&score));
    }

    #[test]
    fn several_navigation_marks_in_one_measure_round_trip() {
        use scores_core::Navigation;
        let song = "{c: Segno}\n[C]a\n{c: To Coda}\n{c: Coda}\n[G]b\n{c: D.S. al Coda}\n";
        let score = ChordProFormat.read(song.as_bytes()).unwrap();
        let navigation: Vec<_> = score.parts[0]
            .measures
            .iter()
            .map(|m| m.navigation.clone())
            .collect();
        assert_eq!(
            navigation,
            vec![
                vec![Navigation::Segno, Navigation::ToCoda],
                vec![Navigation::Coda, Navigation::DalSegnoAlCoda],
            ]
        );
        let written = ChordProFormat.write(&score).unwrap();
        let reread = ChordProFormat.read(&written).unwrap();
        assert_eq!(reread.parts[0].measures, score.parts[0].measures);
    }
}
