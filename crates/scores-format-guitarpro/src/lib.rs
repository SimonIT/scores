//! Guitar Pro plugin, on top of the [`guitarpro`] crate's binary GP3/GP4/GP5
//! reader/writer (its `model::legacy` object model). GP6 (`.gpx`) and GP7+
//! (`.gp`) are a different, zip/XML-based format the same crate also
//! supports, but are out of scope here; only the classic binary format is
//! handled, and writing always produces GP5.
//!
//! Guitar Pro is fundamentally tablature: a note is a `(string, fret)` pair
//! played on a track's tuned strings, not a notated pitch. Mapping that onto
//! [`scores_core::Pitch`] is exact in one direction (string+fret+tuning ->
//! MIDI number, reimplementing the crate's own (private) `Note::real_value`)
//! and a *choice* in the other (which string/fret plays a given pitch);
//! this picks the highest string that can play each note in frets 0-24,
//! falling back to the closest clamped fret if none can.
//!
//! Both of a measure's (up to 2) voices are read/written. Beyond pitches,
//! rhythm, ties, key/time signatures and repeats, these round trip:
//!
//! - **Dynamics**: Guitar Pro has no dynamic markings, only a per-note
//!   velocity in 8 steps (ppp..fff). Reading emits a [`scores_core::Dynamic`]
//!   wherever a voice's velocity changes (tied continuations ignored),
//!   starting from Guitar Pro's default f - so an explicit leading f is
//!   indistinguishable from none. Writing plays every note at the latest
//!   dynamic's level; momentary/compound markings (sf, sfz, fz, rf, fp,
//!   sfp, ...) become one louder note that then falls back (fp-style ones:
//!   to p/pp), so they read back as plain levels.
//! - **Articulations**: accent, strong (heavy) accent, staccato, and the
//!   slide in/out effects as scoop (from below), plop (from above), doit
//!   (out upwards) and falloff (out downwards).
//! - **Ornaments**: trill mark (written as a whole-step trill), tremolo
//!   picking as `Tremolo(1..=3)` (8th/16th/32nd speed), vibrato as a wavy
//!   line (a wavy line next to a trill mark is taken as the trill's own
//!   line and not written as vibrato).
//! - **Grace notes**: a Guitar Pro grace is an effect on its main note (same
//!   string, own fret), read as a separate grace event before it. Only
//!   16th/triplet-16th/32nd grace lengths exist (longer ones are written as
//!   16ths), one grace per main note (the last of several consecutive grace
//!   events wins), it must be playable on its main note's string, and
//!   graces before a rest or at the end of a measure are dropped.
//! - **Chord symbols**: a beat's chord diagram, as a name-only chord (no
//!   fingering diagram) whose name is our shared text form, with the
//!   root/type/extension/bass fields filled in as far as Guitar Pro's
//!   chord vocabulary reaches. Reading prefers the name and falls back to
//!   those structured fields.
//! - **Endings**: `repeat_alternative` (a bitmask on every measure under a
//!   bracket) <-> one [`Ending`](scores_core::Ending) per run of measures
//!   sharing a mask, spanning the run. Guitar Pro has no closed/open hook,
//!   so a bracket reads as closed exactly when its last measure has a
//!   repeat-end (and that flag is dropped on writing).
//! - **Barline style**: only `double_bar` exists, read as light-light and
//!   written for light-light/heavy-heavy; every other style is written
//!   as a regular barline.
//! - **Navigation** (GP5 only; GP3/4 have no such marks): segno, coda, fine,
//!   D.C./D.S. (al fine/al coda) and "Da Coda" as to-coda. GP5's
//!   "double coda"/"segno segno" variants read as their single
//!   counterparts, and each sign can appear only once per song. A measure
//!   holds at most one direction, so only its first navigation mark is
//!   written.
//!
//! Unsupported: fermatas and hairpins (the binary format has neither -
//! "fade in" is a playback volume swell, not a notated crescendo), the
//! other articulations/ornaments (tenuto, mordents, turns, ...), and the
//! guitar-specific techniques with no equivalent in our model
//! (`bend`/`harmonic`/shift and legato slides/`hammer`/`palm_mute`/
//! `let_ring`/`ghost_note`/fingerings/chord diagrams' frets), which are
//! dropped. "Dead"/muted notes are read as ordinary pitched notes.
//! Repeats/endings/barlines/navigation are shared by all tracks in Guitar
//! Pro, so on writing the first part that has them wins.

mod convert;
mod read;
mod write;

use scores_core::{Format, FormatReader, FormatWriter, Result, Score};

pub struct GuitarProFormat;

impl FormatReader for GuitarProFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for GuitarProFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for GuitarProFormat {
    fn id(&self) -> &'static str {
        "guitarpro"
    }

    fn name(&self) -> &'static str {
        "Guitar Pro"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["gp5", "gp4", "gp3"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::{
        Articulation, Barline, BarlineType, ChordKind, ChordRoot, ChordSymbol, Duration, Dynamic,
        Ending, Event, KeySignature, Measure, MeasureAttributes, Mode, Navigation, Note, NoteValue,
        Ornament, Part, Pitch, Rest, Score, Step, Tempo, Tie, TimeSignature, Tuplet,
    };

    fn quarter(step: Step, octave: i8) -> Note {
        Note::new(
            Pitch::new(step, 0, octave),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        )
    }

    fn round_trip(score: &Score) -> Score {
        let bytes = GuitarProFormat.write(score).unwrap();
        GuitarProFormat.read(&bytes).unwrap()
    }

    fn first_voice(score: &Score, measure: usize) -> &[Event] {
        &score.parts[0].measures[measure].voices[0].events
    }

    fn scale_score() -> Score {
        let notes = [Step::C, Step::D, Step::E, Step::F];
        let events = notes
            .iter()
            .map(|&step| {
                Event::Note(Note::new(
                    Pitch::new(step, 0, 4),
                    Duration::new(NoteValue::Quarter),
                    Tie::None,
                ))
            })
            .collect();

        Score {
            title: Some("Scale".to_string()),
            composer: Some("Anon.".to_string()),
            parts: vec![Part {
                id: "P1".to_string(),
                name: Some("Guitar".to_string()),
                measures: vec![Measure::single_voice(
                    1,
                    MeasureAttributes {
                        time_signature: Some(TimeSignature {
                            beats: 4,
                            beat_type: 4,
                        }),
                        key_signature: Some(KeySignature {
                            fifths: 0,
                            mode: Mode::Major,
                        }),
                        clef: None,
                        tempo: Some(Tempo { bpm: 120.0 }),
                    },
                    events,
                )],
            }],
        }
    }

    #[test]
    fn writes_a_parseable_gp5_file() {
        let bytes = GuitarProFormat.write(&scale_score()).unwrap();
        let mut song = guitarpro::Song::default();
        song.read_gp5(&bytes)
            .expect("guitarpro should parse what we wrote");
        assert_eq!(song.name, "Scale");
    }

    #[test]
    fn round_trips_pitches_and_durations() {
        let original = scale_score();
        let bytes = GuitarProFormat.write(&original).unwrap();
        let reread = GuitarProFormat.read(&bytes).unwrap();

        assert_eq!(reread.title, original.title);
        assert_eq!(reread.composer, original.composer);
        assert_eq!(reread.parts.len(), 1);

        let events = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(events.len(), 4);
        for (original_event, reread_event) in original.parts[0].measures[0].voices[0]
            .events
            .iter()
            .zip(events)
        {
            let (Event::Note(a), Event::Note(b)) = (original_event, reread_event) else {
                panic!()
            };
            assert_eq!(a.pitch.to_midi_number(), b.pitch.to_midi_number());
            assert_eq!(a.duration.quarter_length(), b.duration.quarter_length());
        }
    }

    #[test]
    fn ties_and_chords_round_trip() {
        let mut score = scale_score();
        score.parts[0].measures[0].voices[0].events = vec![
            Event::Chord(vec![
                Note::new(
                    Pitch::new(Step::C, 0, 3),
                    Duration::new(NoteValue::Half),
                    Tie::None,
                ),
                Note::new(
                    Pitch::new(Step::E, 0, 3),
                    Duration::new(NoteValue::Half),
                    Tie::None,
                ),
            ]),
            Event::Note(Note::new(
                Pitch::new(Step::G, 0, 3),
                Duration::new(NoteValue::Quarter),
                Tie::Start,
            )),
            Event::Note(Note::new(
                Pitch::new(Step::G, 0, 3),
                Duration::new(NoteValue::Quarter),
                Tie::Stop,
            )),
        ];

        let bytes = GuitarProFormat.write(&score).unwrap();
        let reread = GuitarProFormat.read(&bytes).unwrap();
        let events = &reread.parts[0].measures[0].voices[0].events;

        let Event::Chord(notes) = &events[0] else {
            panic!("expected a chord")
        };
        assert_eq!(notes.len(), 2);

        let Event::Note(start) = &events[1] else {
            panic!()
        };
        assert_eq!(start.tie, Tie::Start);
        let Event::Note(stop) = &events[2] else {
            panic!()
        };
        assert_eq!(stop.tie, Tie::Stop);
    }

    #[test]
    fn second_voice_round_trips() {
        let mut score = scale_score();
        // Open low E string - guaranteed playable, so fret-assignment
        // fallback clamping can't change the pitch out from under the test.
        let bass_events = vec![Event::Note(Note::new(
            Pitch::new(Step::E, 0, 2),
            Duration::new(NoteValue::Whole),
            Tie::None,
        ))];
        score.parts[0].measures[0].voices.push(scores_core::Voice {
            number: 2,
            events: bass_events,
        });

        let bytes = GuitarProFormat.write(&score).unwrap();
        let reread = GuitarProFormat.read(&bytes).unwrap();
        let voices = &reread.parts[0].measures[0].voices;
        assert_eq!(voices.len(), 2);

        let voice2 = voices.iter().find(|v| v.number == 2).unwrap();
        let Event::Note(bass) = &voice2.events[0] else {
            panic!()
        };
        assert_eq!(bass.pitch.step, Step::E);
        assert_eq!(bass.pitch.octave, 2);
    }

    #[test]
    fn note_effects_and_repeats_round_trip() {
        let mut score = scale_score();
        score.parts[0].measures[0].barline.repeat_start = true;
        score.parts[0].measures[0].barline.repeat_end = Some(3);

        let mut accented = Note::new(
            Pitch::new(Step::G, 0, 4),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        );
        accented.articulations.push(Articulation::Accent);
        accented.ornaments.push(Ornament::TrillMark);
        score.parts[0].measures[0].voices[0]
            .events
            .push(Event::Note(accented));

        let bytes = GuitarProFormat.write(&score).unwrap();
        let reread = GuitarProFormat.read(&bytes).unwrap();
        let measure = &reread.parts[0].measures[0];

        assert!(measure.barline.repeat_start);
        assert_eq!(measure.barline.repeat_end, Some(3));

        let Event::Note(last) = measure.voices[0].events.last().unwrap() else {
            panic!()
        };
        assert!(last.articulations.contains(&Articulation::Accent));
        assert!(last.ornaments.contains(&Ornament::TrillMark));
    }

    #[test]
    fn dynamics_round_trip_as_velocity_changes() {
        let mut score = scale_score();
        let mut notes: Vec<Note> = [Step::C, Step::D, Step::E, Step::F]
            .iter()
            .map(|&step| quarter(step, 4))
            .collect();
        notes[0].dynamic = Some(Dynamic::P);
        notes[2].dynamic = Some(Dynamic::Mf);
        notes[3].dynamic = Some(Dynamic::Sfz);
        let mut after = quarter(Step::G, 4);
        after.dynamic = None;
        score.parts[0].measures[0].voices[0].events = notes.into_iter().map(Event::Note).collect();
        score.parts[0].measures.push(Measure::single_voice(
            2,
            MeasureAttributes::default(),
            vec![Event::Note(after), Event::Note(quarter(Step::A, 4))],
        ));

        let reread = round_trip(&score);
        let dynamics: Vec<Option<Dynamic>> = (0..2)
            .flat_map(|m| first_voice(&reread, m).iter())
            .map(|e| match e {
                Event::Note(n) => n.dynamic.clone(),
                _ => panic!("expected notes only"),
            })
            .collect();
        // sfz becomes a single fff note, then the level falls back to mf -
        // carried across the barline.
        assert_eq!(
            dynamics,
            vec![
                Some(Dynamic::P),
                None,
                Some(Dynamic::Mf),
                Some(Dynamic::Fff),
                Some(Dynamic::Mf),
                None
            ]
        );
    }

    #[test]
    fn no_dynamics_stay_unmarked() {
        let reread = round_trip(&scale_score());
        assert!(first_voice(&reread, 0).iter().all(|e| match e {
            Event::Note(n) => n.dynamic.is_none(),
            _ => true,
        }));
    }

    #[test]
    fn articulations_and_ornaments_round_trip() {
        let mut score = scale_score();
        let mut a = quarter(Step::C, 4);
        a.articulations = vec![Articulation::StrongAccent, Articulation::Staccato];
        a.ornaments = vec![Ornament::Tremolo(2)];
        let mut b = quarter(Step::D, 4);
        b.articulations = vec![Articulation::Scoop, Articulation::Falloff];
        b.ornaments = vec![Ornament::WavyLine];
        let mut c = quarter(Step::E, 4);
        c.articulations = vec![Articulation::Plop, Articulation::Doit];
        c.ornaments = vec![Ornament::TrillMark, Ornament::Tremolo(3)];
        let mut d = quarter(Step::F, 4);
        d.ornaments = vec![Ornament::Tremolo(1)];
        let expected = vec![a, b, c, d];
        score.parts[0].measures[0].voices[0].events =
            expected.iter().cloned().map(Event::Note).collect();

        let reread = round_trip(&score);
        let events = first_voice(&reread, 0);
        assert_eq!(events.len(), expected.len());
        for (event, want) in events.iter().zip(&expected) {
            let Event::Note(got) = event else { panic!() };
            let mut got_articulations = got.articulations.clone();
            let mut want_articulations = want.articulations.clone();
            got_articulations.sort_by_key(|a| format!("{a:?}"));
            want_articulations.sort_by_key(|a| format!("{a:?}"));
            assert_eq!(got_articulations, want_articulations);
            assert_eq!(got.ornaments, want.ornaments);
        }
    }

    #[test]
    fn grace_notes_round_trip() {
        let mut score = scale_score();
        let mut grace = Note::new(
            Pitch::new(Step::E, 0, 4),
            Duration::new(NoteValue::Sixteenth),
            Tie::None,
        );
        grace.grace = true;
        let mut grace_32nd = Note::new(
            Pitch::new(Step::A, 0, 4),
            Duration::new(NoteValue::ThirtySecond),
            Tie::None,
        );
        grace_32nd.grace = true;
        let mut grace_24th = Note::new(
            Pitch::new(Step::B, 0, 4),
            Duration {
                base: NoteValue::Sixteenth,
                dots: 0,
                tuplet: Some(Tuplet {
                    actual: 3,
                    normal: 2,
                }),
            },
            Tie::None,
        );
        grace_24th.grace = true;
        let expected = vec![
            Event::Note(grace),
            Event::Note(quarter(Step::F, 4)),
            Event::Note(quarter(Step::G, 4)),
            Event::Note(grace_32nd),
            Event::Note(quarter(Step::G, 4)),
            Event::Note(grace_24th),
            Event::Note(quarter(Step::C, 5)),
            Event::Rest(Rest::new(Duration::new(NoteValue::Quarter))),
        ];
        score.parts[0].measures[0].voices[0].events = expected.clone();

        let reread = round_trip(&score);
        assert_eq!(first_voice(&reread, 0), expected.as_slice());
    }

    #[test]
    fn chord_symbols_round_trip() {
        let symbol = |text: &str| ChordSymbol::parse(text).unwrap();
        let mut score = scale_score();
        let mut events = vec![
            Event::Note(quarter(Step::C, 4)),
            Event::Rest(Rest::new(Duration::new(NoteValue::Quarter))),
            Event::Chord(vec![quarter(Step::C, 4), quarter(Step::E, 4)]),
            Event::Note(quarter(Step::G, 4)),
        ];
        events[0].set_chord_symbol(Some(symbol("Cmaj7")));
        events[1].set_chord_symbol(Some(symbol("Am/G")));
        events[2].set_chord_symbol(Some(symbol("F#m7b5")));
        events[3].set_chord_symbol(Some(symbol("G7b9")));
        score.parts[0].measures[0].voices[0].events = events.clone();

        let reread = round_trip(&score);
        let got: Vec<_> = first_voice(&reread, 0)
            .iter()
            .map(|e| e.chord_symbol().cloned())
            .collect();
        let want: Vec<_> = events.iter().map(|e| e.chord_symbol().cloned()).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn unnamed_chords_fall_back_to_structured_fields() {
        let symbol = ChordSymbol {
            root: ChordRoot::new(Step::B, -1),
            kind: ChordKind::MinorNinth,
            bass: Some(ChordRoot::new(Step::F, 0)),
        };
        let mut chord = convert::symbol_to_chord(&symbol);
        chord.name.clear();
        assert_eq!(convert::chord_to_symbol(&chord), Some(symbol));
    }

    #[test]
    fn endings_barlines_and_navigation_round_trip() {
        let mut score = scale_score();
        let template = score.parts[0].measures[0].clone();
        let barlines = [
            Barline {
                repeat_start: true,
                ..Barline::default()
            },
            // A closed three-measure 1st ending...
            Barline {
                ending: Some(Ending {
                    numbers: vec![1],
                    measures: 3,
                    closed: true,
                }),
                ..Barline::default()
            },
            Barline::default(),
            Barline {
                repeat_end: Some(2),
                ..Barline::default()
            },
            // ...and an open one-measure 2nd ending.
            Barline {
                kind: BarlineType::LightLight,
                ending: Some(Ending {
                    numbers: vec![2],
                    measures: 1,
                    closed: false,
                }),
                ..Barline::default()
            },
            Barline::default(),
            Barline {
                repeat_start: true,
                ..Barline::default()
            },
            // A closed one-measure 1st ending...
            Barline {
                repeat_end: Some(2),
                ending: Some(Ending {
                    numbers: vec![1],
                    measures: 1,
                    closed: true,
                }),
                ..Barline::default()
            },
            // ...and an open two-measure 2nd ending.
            Barline {
                ending: Some(Ending {
                    numbers: vec![2],
                    measures: 2,
                    closed: false,
                }),
                ..Barline::default()
            },
            Barline::default(),
        ];
        let navigation = [
            vec![Navigation::Segno],
            vec![],
            vec![Navigation::ToCoda],
            vec![],
            vec![Navigation::DalSegnoAlCoda],
            vec![Navigation::Coda],
            vec![],
            vec![],
            vec![],
            vec![],
        ];
        score.parts[0].measures = barlines
            .iter()
            .zip(navigation.clone())
            .enumerate()
            .map(|(i, (barline, navigation))| Measure {
                number: i as u32 + 1,
                attributes: if i == 0 {
                    template.attributes.clone()
                } else {
                    MeasureAttributes::default()
                },
                barline: barline.clone(),
                navigation,
                ..template.clone()
            })
            .collect();

        let bytes = GuitarProFormat.write(&score).unwrap();
        let mut song = guitarpro::Song::default();
        song.read_gp5(&bytes).unwrap();
        let masks: Vec<u8> = song
            .measure_headers
            .iter()
            .map(|h| h.repeat_alternative)
            .collect();
        assert_eq!(masks, vec![0, 1, 1, 1, 2, 0, 0, 1, 2, 2]);
        // Guitar Pro counts repeats, not playthroughs.
        assert_eq!(song.measure_headers[3].repeat_close, 1);

        let reread = GuitarProFormat.read(&bytes).unwrap();
        for (i, measure) in reread.parts[0].measures.iter().enumerate() {
            assert_eq!(measure.barline, barlines[i], "barline of measure {}", i + 1);
            assert_eq!(
                measure.navigation,
                navigation[i],
                "navigation of measure {}",
                i + 1
            );
        }
    }

    #[test]
    fn only_the_first_navigation_mark_of_a_measure_is_written() {
        let mut score = scale_score();
        score.parts[0].measures[0].navigation = vec![Navigation::Segno, Navigation::ToCoda];
        let bytes = GuitarProFormat.write(&score).unwrap();
        let reread = GuitarProFormat.read(&bytes).unwrap();
        assert_eq!(
            reread.parts[0].measures[0].navigation,
            vec![Navigation::Segno]
        );
    }
}
