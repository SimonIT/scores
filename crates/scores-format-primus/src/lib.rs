//! PriMus plugin, on top of the [`primus_scores`] crate's reader/writer
//! for `.pri` files (the undocumented native format of the PriMus
//! notation program, reverse-engineered by that crate).
//!
//! Each PriMus staff is a [`Part`](scores_core::Part); its voices are the
//! measure's [`Voice`](scores_core::Voice)s. A staff's first voice is always
//! read (its empty measures as rests filling the measure); further voices
//! only where they hold notes. Only the first piece ("Stück") of a
//! multi-piece document is read. Pitches are the written ones, as PriMus
//! stores them (a transposing instrument's transposition is not applied);
//! drum staves read their General MIDI percussion keys as pitches.
//!
//! Writing starts from an embedded template (PriMus Free's own
//! "Singstimme": one treble staff), copying its staff for every further
//! part and adding voices as needed, and sets a piano MIDI sound. PriMus
//! stores a measure's actual length apart from its time signature (for
//! pickups and shortened or lengthened measures) and needs every voice to
//! fill it: each measure is as long as its longest voice (an empty one as
//! its time signature), and shorter voices are filled up with rests.
//! Durations PriMus can't write (more than two dots, tuplets that don't
//! divide into whole ticks) are quantized.
//!
//! Beyond pitches, rhythm (dots, tuplets), chords, ties, rests, key/time
//! signatures, clefs (and clef changes), tempo, title, composer and part
//! names, these are read and written:
//!
//! - **Dynamics**: PriMus's dynamic symbols (p..pppp, f..ffff, mp, mf, fp,
//!   sfp, sfpp, fz, sf, sfz). Others (rf, rfz, sffz, n, ...) are dropped.
//! - **Hairpins**: `SPAN` hairpins, paired with a [`Wedge::Stop`] on the
//!   last note they cover, in the same voice.
//! - **Articulations**: staccato, tenuto, detached legato (tenuto +
//!   staccato), accent, strong accent (the `^` wedge), staccatissimo (the
//!   stroke), spiccato (the wedge), scoop, plop, falloff, doit, breath mark
//!   and caesura.
//! - **Ornaments**: trill mark, wavy line (trill wave), mordent, inverted
//!   mordent (PriMus's short trill; [`Ornament::Shake`] is written as it
//!   too), turn and single-note tremolos. PriMus's symbols belong to a
//!   whole chord: a chord's marks are merged on write and put on its first
//!   note on read.
//! - **Fermatas** on notes and rests.
//! - **Grace notes**, as PriMus grace notes (unslashed). One not followed by
//!   a note in its measure is dropped.
//! - **Chord symbols** above notes and rests, as far as PriMus's chord
//!   types reach (see [`primus_scores::ChordSymbol::parse`]); N.C. and
//!   others it can't name are dropped.
//! - **Repeats and endings**: repeat barlines and "1." / "2." brackets
//!   (repeat counts other than 2 are not expressible; a repeat start at the
//!   very first measure is implicit in PriMus).
//! - **Barline style**: double, heavy-light, final, invisible, dotted,
//!   dashed.
//! - **Navigation**: D.C./D.S. (al Fine/al Coda), Fine, To Coda and Coda as
//!   free measure texts (read when a text spells one), the segno as
//!   PriMus's segno symbol on the measure's first note (or a text).
//!
//! Lyrics, slurs, fingerings and PriMus's layout (positions, stems, beams,
//! systems) have no place in the shared model and are dropped.
//!
//! [`Wedge::Stop`]: scores_core::Wedge::Stop
//! [`Ornament::Shake`]: scores_core::Ornament::Shake

mod convert;
mod read;
mod write;

use scores_core::{Format, FormatReader, FormatWriter, Result, Score};

pub struct PrimusFormat;

impl FormatReader for PrimusFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for PrimusFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for PrimusFormat {
    fn id(&self) -> &'static str {
        "primus"
    }

    fn name(&self) -> &'static str {
        "PriMus"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["pri"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::{
        Articulation, Barline, BarlineType, ChordSymbol, Clef, Duration, Dynamic, Ending, Event,
        KeySignature, Measure, MeasureAttributes, Mode, Navigation, Note, NoteValue, Ornament,
        Part, Pitch, Rest, Step, Tempo, Tie, TimeSignature, Tuplet, Voice, Wedge,
    };

    fn note(step: Step, octave: i8, value: NoteValue) -> Note {
        Note::new(Pitch::new(step, 0, octave), Duration::new(value), Tie::None)
    }

    fn quarter(step: Step, octave: i8) -> Event {
        Event::Note(note(step, octave, NoteValue::Quarter))
    }

    fn attributes() -> MeasureAttributes {
        MeasureAttributes {
            time_signature: Some(TimeSignature {
                beats: 4,
                beat_type: 4,
            }),
            key_signature: Some(KeySignature {
                fifths: 0,
                mode: Mode::Major,
            }),
            clef: Some(Clef::Treble),
            tempo: Some(Tempo { bpm: 120.0 }),
        }
    }

    fn score(measures: Vec<Measure>) -> Score {
        Score {
            title: Some("Scale".into()),
            composer: Some("Anon.".into()),
            parts: vec![Part {
                id: "P1".into(),
                name: Some("Flöte".into()),
                measures,
            }],
        }
    }

    fn scale() -> Score {
        let steps = [Step::C, Step::D, Step::E, Step::F];
        score(vec![Measure::single_voice(
            1,
            attributes(),
            steps.iter().map(|&s| quarter(s, 4)).collect(),
        )])
    }

    fn round_trip(score: &Score) -> Score {
        let bytes = PrimusFormat.write(score).unwrap();
        PrimusFormat.read(&bytes).unwrap()
    }

    fn events(score: &Score, part: usize, measure: usize) -> &[Event] {
        &score.parts[part].measures[measure].voices[0].events
    }

    #[test]
    fn reads_what_primus_scores_writes() {
        let bytes = PrimusFormat.write(&scale()).unwrap();
        let file = primus_scores::PriFile::from_bytes(&bytes).unwrap();
        assert_eq!(file.document.voice_notes(0).unwrap().len(), 4);
    }

    #[test]
    fn round_trips_metadata_and_pitches() {
        let back = round_trip(&scale());
        assert_eq!(back.title.as_deref(), Some("Scale"));
        assert_eq!(back.composer.as_deref(), Some("Anon."));
        assert_eq!(back.parts[0].name.as_deref(), Some("Flöte"));
        let first = &back.parts[0].measures[0];
        assert_eq!(first.attributes.time_signature, attributes().time_signature);
        assert_eq!(first.attributes.clef, Some(Clef::Treble));
        assert_eq!(first.attributes.tempo, Some(Tempo { bpm: 120.0 }));
        assert_eq!(events(&back, 0, 0), events(&scale(), 0, 0));
    }

    #[test]
    fn round_trips_rhythm_chords_and_ties() {
        let mut dotted = note(Step::G, 4, NoteValue::Quarter);
        dotted.duration = Duration::dotted(NoteValue::Quarter, 1);
        let triplet = |step| {
            let mut n = note(step, 5, NoteValue::Eighth);
            n.duration.tuplet = Some(Tuplet {
                actual: 3,
                normal: 2,
            });
            Event::Note(n)
        };
        let mut tied = note(Step::B, 4, NoteValue::Eighth);
        tied.tie = Tie::Start;
        let mut sharp = note(Step::F, 4, NoteValue::Half);
        sharp.pitch.alter = 1;
        let mut end = note(Step::B, 4, NoteValue::Half);
        end.tie = Tie::Stop;
        let measures = vec![
            Measure::single_voice(
                1,
                attributes(),
                vec![
                    Event::Note(dotted),
                    triplet(Step::C),
                    triplet(Step::D),
                    triplet(Step::E),
                    Event::Rest(Rest::new(Duration::new(NoteValue::Quarter))),
                    Event::Note(tied),
                ],
            ),
            Measure::single_voice(
                2,
                MeasureAttributes::default(),
                vec![
                    Event::Note(end),
                    Event::Chord(vec![note(Step::C, 4, NoteValue::Half), sharp]),
                ],
            ),
        ];
        let original = score(measures);
        let back = round_trip(&original);
        assert_eq!(events(&back, 0, 0), events(&original, 0, 0));
        assert_eq!(events(&back, 0, 1), events(&original, 0, 1));
    }

    #[test]
    fn keeps_short_measures_and_fills_empty_ones() {
        let measures = vec![
            Measure::single_voice(1, attributes(), vec![quarter(Step::C, 4)]),
            Measure::single_voice(2, MeasureAttributes::default(), vec![quarter(Step::D, 4)]),
            Measure::single_voice(3, MeasureAttributes::default(), vec![]),
        ];
        let back = round_trip(&score(measures));
        // Short measures stay short, as PriMus stores them.
        assert_eq!(events(&back, 0, 0), [quarter(Step::C, 4)]);
        assert_eq!(events(&back, 0, 1), [quarter(Step::D, 4)]);
        assert!(matches!(events(&back, 0, 2), [Event::Rest(_)]));
    }

    #[test]
    fn round_trips_marks() {
        let mut first = note(Step::C, 4, NoteValue::Quarter);
        first.dynamic = Some(Dynamic::Mf);
        first.articulations = vec![Articulation::Staccato, Articulation::Accent];
        first.wedge = Some(Wedge::CrescendoStart);
        first.chord_symbol = ChordSymbol::parse("Am7");
        let mut second = note(Step::D, 4, NoteValue::Quarter);
        second.ornaments = vec![Ornament::TrillMark];
        let mut third = note(Step::E, 4, NoteValue::Quarter);
        third.wedge = Some(Wedge::Stop);
        third.articulations = vec![Articulation::DetachedLegato];
        let mut fourth = note(Step::F, 4, NoteValue::Quarter);
        fourth.fermata = true;
        let mut grace = note(Step::G, 4, NoteValue::Eighth);
        grace.grace = true;
        let events_in = vec![
            Event::Note(first),
            Event::Note(second),
            Event::Note(grace),
            Event::Note(third),
            Event::Note(fourth),
        ];
        let original = score(vec![Measure::single_voice(1, attributes(), events_in)]);
        let back = round_trip(&original);
        assert_eq!(events(&back, 0, 0), events(&original, 0, 0));
    }

    #[test]
    fn round_trips_repeats_endings_and_navigation() {
        let bar = |n| Measure::single_voice(n, MeasureAttributes::default(), vec![]);
        let mut measures: Vec<Measure> = (1..=5).map(bar).collect();
        measures[0].attributes = attributes();
        measures[1].barline.repeat_start = true;
        measures[1].navigation = vec![Navigation::Segno];
        measures[2].barline = Barline {
            repeat_end: Some(2),
            ending: Some(Ending {
                numbers: vec![1],
                measures: 1,
                closed: true,
            }),
            ..Barline::default()
        };
        measures[3].barline.ending = Some(Ending {
            numbers: vec![2],
            measures: 1,
            closed: false,
        });
        measures[3].navigation = vec![Navigation::DalSegnoAlFine];
        measures[4].barline.kind = BarlineType::LightHeavy;
        measures[4].navigation = vec![Navigation::Fine];
        let back = round_trip(&score(measures.clone()));
        for (a, b) in measures.iter().zip(&back.parts[0].measures) {
            assert_eq!(a.barline, b.barline, "measure {}", a.number);
            assert_eq!(a.navigation, b.navigation, "measure {}", a.number);
        }
    }

    #[test]
    fn round_trips_several_parts_and_voices() {
        let mut lower = Measure::single_voice(
            1,
            MeasureAttributes {
                clef: Some(Clef::Bass),
                ..attributes()
            },
            vec![Event::Note(note(Step::C, 3, NoteValue::Whole))],
        );
        lower.voices.push(Voice {
            number: 2,
            events: vec![Event::Note(note(Step::G, 2, NoteValue::Whole))],
        });
        let mut original = scale();
        original.parts.push(Part {
            id: "P2".into(),
            name: Some("Bass".into()),
            measures: vec![lower.clone()],
        });
        original.parts[0].measures[0].attributes.key_signature = Some(KeySignature {
            fifths: -2,
            mode: Mode::Major,
        });
        let back = round_trip(&original);
        assert_eq!(back.parts.len(), 2);
        assert_eq!(back.parts[1].name.as_deref(), Some("Bass"));
        let bass = &back.parts[1].measures[0];
        assert_eq!(bass.attributes.clef, Some(Clef::Bass));
        assert_eq!(bass.attributes.key_signature.map(|k| k.fifths), Some(-2));
        assert_eq!(bass.voices, lower.voices);
    }

    #[test]
    fn rejects_garbage() {
        assert!(PrimusFormat.read(b"not a zip").is_err());
    }
}
