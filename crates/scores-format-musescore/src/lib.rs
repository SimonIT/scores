//! MuseScore plugin, on top of the [`guitarpro`] crate's `.mscz` support
//! (`read_mscz`/`write_mscz` plus its `model::optimized::LoadedScore`
//! object model - a different, richer model than the `model::legacy` one
//! `scores-format-guitarpro` uses for GP3/4/5).
//!
//! `guitarpro` 0.4.3's MSCX <-> `LoadedScore` converter is still an early
//! work in progress (its own module docs call this out): several fields
//! exist in the schema but are unconditionally dropped on read and ignored
//! on write, not merely "not populated yet" in a way this plugin could fix.
//! What actually round-trips today:
//! - Pitch, duration (base note value + dots only - **tuplets are always
//!   dropped**, hard-coded `None` by the converter regardless of what's
//!   written), and ties (pairwise `Start`/`End` only - a note tied to both
//!   neighbours can't be represented and is written as `Start`, dropping
//!   the incoming tie).
//! - Multiple voices per measure.
//! - Tempo, time signature and key signature as a timeline with mid-file
//!   changes (`None` = unchanged since the previous measure, matching this
//!   model's own convention exactly). Key signatures are root+mode in
//!   MuseScore's model; modes beyond major/minor are folded into the
//!   circle-of-fifths count they'd produce (still the correct accidentals,
//!   just without the modal name - see [`convert`]).
//! - Plain repeat-open/repeat-close barlines, with a repeat count.
//!
//! Everything else in scope is dropped by the converter (the schema has
//! fields for some of it, but its read/write code ignores them), so this
//! plugin handles it itself on the raw MSCX (see the `mscx` module): on read, a
//! second pass over the XML is overlaid onto the converter's output by
//! staff/measure/voice/beat index; on write, extra elements are spliced
//! into the XML the converter generated. That covers, read and write:
//! - Dynamics (`<Dynamic><subtype>`; markings MuseScore has no subtype for
//!   use `other-dynamics` + text) and crescendo/diminuendo hairpins
//!   (`<Spanner type="HairPin">`, paired with a `Wedge::Stop` on the last
//!   note they cover, in the same voice - possibly across barlines).
//! - Articulations (`<Articulation>` symbols; falls/doits/plops/scoops as
//!   `<ChordLine>`, breath marks/caesuras as `<Breath>`), ornaments
//!   (`<Ornament>`, or MuseScore 3's `ornament*` articulation symbols;
//!   single-note tremolos as `<Tremolo>`) and fermatas (`<Fermata>`). The
//!   model stores these per note, MuseScore per chord: a chord's marks are
//!   merged on write and put on its first note on read. Delayed turns are
//!   written as plain turns (MuseScore has no delayed turn); wavy-line trills
//!   (a `Trill` spanner) aren't supported.
//! - Grace notes (`<acciaccatura/>`, `<appoggiatura/>`, `<grace4/>`...
//!   all read as grace; written as `<appoggiatura/>`, since the model has
//!   no slash flag).
//! - Chord symbols (`<Harmony>` with `<root>`/`<base>` as tonal pitch
//!   classes and the [`scores_core::ChordKind`] suffix as `<name>`; N.C. as a
//!   root-less `N.C.`). Roman-numeral/Nashville harmony is skipped.
//! - 1st/2nd endings (`<Spanner type="Volta">`, spanning the ending's
//!   measures; `<endHookType>` 1 for a closed bracket, 0 for an open one).
//! - Barline style (`<BarLine><subtype>`; "tick"/"short" via MuseScore's
//!   span presets, no barline as an invisible one), per part.
//! - D.C./D.S./Coda/Segno/Fine/To Coda (`<Jump>`/`<Marker>`, any number
//!   per measure).
//!
//! Voltas and jumps/markers are system-wide in MuseScore: they're written
//! into the first staff only (from the first part that has the measure,
//! like repeats) and read back onto every part. A dynamic or hairpin on a
//! rest can't be represented by the model and is dropped on read. Only
//! MuseScore 4 files (`<museScore version="4.x">`) are accepted - the
//! `guitarpro` parser rejects older versions.

mod convert;
mod mscx;
mod read;
mod write;

use scores_core::{Format, FormatReader, FormatWriter, Result, Score};

pub struct MuseScoreFormat;

impl FormatReader for MuseScoreFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for MuseScoreFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for MuseScoreFormat {
    fn id(&self) -> &'static str {
        "musescore"
    }

    fn name(&self) -> &'static str {
        "MuseScore"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["mscz"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::{
        Articulation, BarlineType, ChordKind, ChordRoot, ChordSymbol, Duration, Dynamic, Ending,
        Event, KeySignature, Measure, MeasureAttributes, Mode, Navigation, Note, NoteValue,
        Ornament, Part, Pitch, Rest, Score, Step, Tempo, Tie, TimeSignature, Voice, Wedge,
    };

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
                name: Some("Piano".to_string()),
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
    fn writes_a_parseable_mscz_file() {
        let bytes = MuseScoreFormat.write(&scale_score()).unwrap();
        assert_eq!(&bytes[0..2], b"PK"); // mscz is a zip archive
        guitarpro::read_mscz_bytes(&bytes).expect("guitarpro should parse what we wrote");
    }

    #[test]
    fn round_trips_pitches_and_durations() {
        let original = scale_score();
        let bytes = MuseScoreFormat.write(&original).unwrap();
        let reread = MuseScoreFormat.read(&bytes).unwrap();

        assert_eq!(reread.title, original.title);
        assert_eq!(reread.composer, original.composer);
        assert_eq!(reread.parts.len(), 1);
        assert_eq!(reread.parts[0].name, original.parts[0].name);

        let first_measure = &reread.parts[0].measures[0];
        assert_eq!(
            first_measure.attributes.time_signature,
            original.parts[0].measures[0].attributes.time_signature
        );
        assert_eq!(
            first_measure.attributes.key_signature,
            original.parts[0].measures[0].attributes.key_signature
        );
        assert_eq!(
            first_measure.attributes.tempo,
            original.parts[0].measures[0].attributes.tempo
        );

        let events = &first_measure.voices[0].events;
        assert_eq!(events.len(), 4);
        for (original_event, reread_event) in original.parts[0].measures[0].voices[0]
            .events
            .iter()
            .zip(events)
        {
            let (Event::Note(a), Event::Note(b)) = (original_event, reread_event) else {
                panic!()
            };
            assert_eq!(a.pitch, b.pitch);
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

        let bytes = MuseScoreFormat.write(&score).unwrap();
        let reread = MuseScoreFormat.read(&bytes).unwrap();
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
        let bass_events = vec![Event::Note(Note::new(
            Pitch::new(Step::C, 0, 3),
            Duration::new(NoteValue::Whole),
            Tie::None,
        ))];
        score.parts[0].measures[0].voices.push(Voice {
            number: 2,
            events: bass_events,
        });

        let bytes = MuseScoreFormat.write(&score).unwrap();
        let reread = MuseScoreFormat.read(&bytes).unwrap();
        let voices = &reread.parts[0].measures[0].voices;
        assert_eq!(voices.len(), 2);

        let voice2 = voices.iter().find(|v| v.number == 2).unwrap();
        let Event::Note(bass) = &voice2.events[0] else {
            panic!()
        };
        assert_eq!(bass.pitch.step, Step::C);
        assert_eq!(bass.pitch.octave, 3);
    }

    #[test]
    fn mid_file_tempo_and_time_signature_changes_round_trip() {
        let mut score = scale_score();
        let second_measure = Measure::single_voice(
            2,
            MeasureAttributes {
                time_signature: Some(TimeSignature {
                    beats: 3,
                    beat_type: 4,
                }),
                key_signature: None,
                clef: None,
                tempo: Some(Tempo { bpm: 90.0 }),
            },
            vec![
                Event::Note(Note::new(
                    Pitch::new(Step::G, 0, 4),
                    Duration::new(NoteValue::Quarter),
                    Tie::None,
                )),
                Event::Note(Note::new(
                    Pitch::new(Step::A, 0, 4),
                    Duration::new(NoteValue::Quarter),
                    Tie::None,
                )),
                Event::Note(Note::new(
                    Pitch::new(Step::B, 0, 4),
                    Duration::new(NoteValue::Quarter),
                    Tie::None,
                )),
            ],
        );
        score.parts[0].measures.push(second_measure);

        let bytes = MuseScoreFormat.write(&score).unwrap();
        let reread = MuseScoreFormat.read(&bytes).unwrap();
        let measures = &reread.parts[0].measures;

        assert_eq!(measures.len(), 2);
        assert_eq!(measures[1].attributes.tempo, Some(Tempo { bpm: 90.0 }));
        assert_eq!(
            measures[1].attributes.time_signature,
            Some(TimeSignature {
                beats: 3,
                beat_type: 4
            })
        );
        assert_eq!(measures[1].voices[0].events.len(), 3);
    }

    #[test]
    fn repeats_round_trip() {
        let mut score = scale_score();
        score.parts[0].measures[0].barline.repeat_start = true;
        score.parts[0].measures[0].barline.repeat_end = Some(3);

        let bytes = MuseScoreFormat.write(&score).unwrap();
        let reread = MuseScoreFormat.read(&bytes).unwrap();
        let barline = &reread.parts[0].measures[0].barline;

        assert!(barline.repeat_start);
        assert_eq!(barline.repeat_end, Some(3));
    }

    fn quarter(step: Step) -> Note {
        Note::new(
            Pitch::new(step, 0, 4),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        )
    }

    /// `scale_score` padded to `count` identical 4/4 measures.
    fn score_with_measures(count: u32) -> Score {
        let mut score = scale_score();
        for number in 2..=count {
            let events = [Step::G, Step::A, Step::B, Step::C]
                .into_iter()
                .map(|s| Event::Note(quarter(s)))
                .collect();
            score.parts[0].measures.push(Measure::single_voice(
                number,
                MeasureAttributes::default(),
                events,
            ));
        }
        score
    }

    fn note_mut(score: &mut Score, measure: usize, index: usize) -> &mut Note {
        match &mut score.parts[0].measures[measure].voices[0].events[index] {
            Event::Note(note) => note,
            Event::Chord(notes) => &mut notes[0],
            Event::Rest(_) => panic!("expected a note"),
        }
    }

    fn note_at(score: &Score, measure: usize, index: usize) -> &Note {
        match &score.parts[0].measures[measure].voices[0].events[index] {
            Event::Note(note) => note,
            Event::Chord(notes) => &notes[0],
            Event::Rest(_) => panic!("expected a note"),
        }
    }

    fn round_trip(score: &Score) -> Score {
        let bytes = MuseScoreFormat.write(score).unwrap();
        MuseScoreFormat.read(&bytes).unwrap()
    }

    /// Archives a hand-written MSCX document the way MuseScore would.
    fn mscz_from_mscx(xml: &str) -> Vec<u8> {
        let mscx = guitarpro::io::mscz::parse_mscx(xml).expect("fixture should parse");
        let container = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container><rootfiles><rootfile full-path=\"score.mscx\"/></rootfiles></container>\n";
        let archive = guitarpro::MsczArchive {
            rootfiles: vec!["score.mscx".to_string()],
            entries: vec![
                guitarpro::MsczEntry {
                    path: "META-INF/container.xml".to_string(),
                    data: container.as_bytes().to_vec(),
                },
                guitarpro::MsczEntry {
                    path: "score.mscx".to_string(),
                    data: xml.as_bytes().to_vec(),
                },
            ],
        };
        guitarpro::write_mscz(&guitarpro::MsczFile { archive, mscx }).unwrap()
    }

    #[test]
    fn dynamics_and_hairpins_round_trip() {
        let mut score = score_with_measures(2);
        note_mut(&mut score, 0, 0).dynamic = Some(Dynamic::Mf);
        note_mut(&mut score, 0, 0).wedge = Some(Wedge::CrescendoStart);
        note_mut(&mut score, 0, 1).wedge = Some(Wedge::Stop);
        // Crosses the barline, and uses dynamics MuseScore has no subtype for.
        note_mut(&mut score, 0, 2).dynamic = Some(Dynamic::Sfzp);
        note_mut(&mut score, 0, 2).wedge = Some(Wedge::DiminuendoStart);
        note_mut(&mut score, 1, 1).wedge = Some(Wedge::Stop);
        note_mut(&mut score, 1, 2).dynamic = Some(Dynamic::Other("sempre p".into()));

        let reread = round_trip(&score);
        assert_eq!(note_at(&reread, 0, 0).dynamic, Some(Dynamic::Mf));
        assert_eq!(note_at(&reread, 0, 0).wedge, Some(Wedge::CrescendoStart));
        assert_eq!(note_at(&reread, 0, 1).wedge, Some(Wedge::Stop));
        assert_eq!(note_at(&reread, 0, 2).dynamic, Some(Dynamic::Sfzp));
        assert_eq!(note_at(&reread, 0, 2).wedge, Some(Wedge::DiminuendoStart));
        assert_eq!(note_at(&reread, 0, 3).wedge, None);
        assert_eq!(note_at(&reread, 1, 1).wedge, Some(Wedge::Stop));
        assert_eq!(
            note_at(&reread, 1, 2).dynamic,
            Some(Dynamic::Other("sempre p".into()))
        );
    }

    #[test]
    fn hairpin_locations_pair_up() {
        // MuseScore matches a spanner's start and end elements by their
        // relative locations, so they must agree with the actual positions.
        let mut score = score_with_measures(2);
        note_mut(&mut score, 0, 1).wedge = Some(Wedge::CrescendoStart);
        note_mut(&mut score, 1, 0).wedge = Some(Wedge::Stop);
        let bytes = MuseScoreFormat.write(&score).unwrap();
        let file = guitarpro::read_mscz_bytes(&bytes).unwrap();
        let xml = &file.mscx.raw_xml;
        // Start at beat 2 of bar 1, end after beat 1 of bar 2: +1 measure, +0.
        assert!(xml.contains("<next><location><measures>1</measures></location></next>"));
        assert!(xml.contains("<prev><location><measures>-1</measures></location></prev>"));
    }

    #[test]
    fn articulations_ornaments_and_fermata_round_trip() {
        let mut score = scale_score();
        {
            let events = &mut score.parts[0].measures[0].voices[0].events;
            events[3] = Event::Rest(Rest {
                fermata: true,
                ..Rest::new(Duration::new(NoteValue::Quarter))
            });
        }
        note_mut(&mut score, 0, 0).articulations = vec![
            Articulation::Staccato,
            Articulation::Accent,
            Articulation::StrongAccent,
            Articulation::Tenuto,
            Articulation::DetachedLegato,
            Articulation::Staccatissimo,
            Articulation::Spiccato,
            Articulation::Stress,
            Articulation::Unstress,
            Articulation::SoftAccent,
        ];
        note_mut(&mut score, 0, 1).ornaments = vec![
            Ornament::TrillMark,
            Ornament::Turn,
            Ornament::InvertedTurn,
            Ornament::VerticalTurn,
            Ornament::InvertedVerticalTurn,
            Ornament::Shake,
            Ornament::Mordent,
            Ornament::InvertedMordent,
            Ornament::Schleifer,
            Ornament::Haydn,
            Ornament::Tremolo(2),
        ];
        let third = note_mut(&mut score, 0, 2);
        third.fermata = true;
        third.articulations = vec![
            Articulation::Falloff,
            Articulation::Doit,
            Articulation::Plop,
            Articulation::Scoop,
            Articulation::Other("brassMuteClosed".into()),
            Articulation::BreathMark,
        ];

        let reread = round_trip(&score);
        let original = &score.parts[0].measures[0].voices[0].events;
        let events = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(events[0], original[0]);
        assert_eq!(events[1], original[1]);
        assert_eq!(events[2], original[2]);
        assert_eq!(events[3], original[3]);
    }

    #[test]
    fn grace_notes_round_trip() {
        let mut score = scale_score();
        let mut grace = Note::new(
            Pitch::new(Step::B, 0, 3),
            Duration::new(NoteValue::Eighth),
            Tie::None,
        );
        grace.grace = true;
        score.parts[0].measures[0].voices[0]
            .events
            .insert(0, Event::Note(grace.clone()));

        let reread = round_trip(&score);
        let events = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(events.len(), 5);
        assert_eq!(events[0], Event::Note(grace));
        assert!(!note_at(&reread, 0, 1).grace);
    }

    #[test]
    fn chord_symbols_round_trip() {
        let mut score = scale_score();
        let events = &mut score.parts[0].measures[0].voices[0].events;
        events[0].set_chord_symbol(Some(ChordSymbol {
            root: ChordRoot::new(Step::C, 0),
            kind: ChordKind::MinorSeventh,
            bass: Some(ChordRoot::new(Step::E, -1)),
        }));
        events[1] = Event::Chord(vec![quarter(Step::B), quarter(Step::D)]);
        events[1].set_chord_symbol(Some(ChordSymbol::new(
            ChordRoot::new(Step::B, -1),
            ChordKind::MajorSeventh,
        )));
        events[2].set_chord_symbol(Some(ChordSymbol::new(
            ChordRoot::new(Step::F, 1),
            ChordKind::Major,
        )));
        events[3] = Event::Rest(Rest::new(Duration::new(NoteValue::Quarter)));
        events[3].set_chord_symbol(Some(ChordSymbol::no_chord()));

        let reread = round_trip(&score);
        let original = &score.parts[0].measures[0].voices[0].events;
        let events = &reread.parts[0].measures[0].voices[0].events;
        for (a, b) in original.iter().zip(events) {
            assert_eq!(a.chord_symbol(), b.chord_symbol());
        }
    }

    #[test]
    fn voltas_round_trip() {
        let mut score = score_with_measures(5);
        let measures = &mut score.parts[0].measures;
        measures[0].barline.repeat_start = true;
        // A one-measure closed first ending, a two-measure open second
        // ending, then a one-measure closed ending on the last measure.
        measures[1].barline.repeat_end = Some(2);
        measures[1].barline.ending = Some(Ending {
            numbers: vec![1],
            measures: 1,
            closed: true,
        });
        measures[2].barline.ending = Some(Ending {
            numbers: vec![2, 3],
            measures: 2,
            closed: false,
        });
        measures[4].barline.ending = Some(Ending {
            numbers: vec![4],
            measures: 1,
            closed: true,
        });

        let reread = round_trip(&score);
        let endings: Vec<_> = reread.parts[0]
            .measures
            .iter()
            .map(|m| m.barline.ending.clone())
            .collect();
        let expected: Vec<_> = score.parts[0]
            .measures
            .iter()
            .map(|m| m.barline.ending.clone())
            .collect();
        assert_eq!(endings, expected);
    }

    #[test]
    fn barline_styles_round_trip() {
        let kinds = [
            BarlineType::Regular,
            BarlineType::Dotted,
            BarlineType::Dashed,
            BarlineType::Heavy,
            BarlineType::LightLight,
            BarlineType::LightHeavy,
            BarlineType::HeavyLight,
            BarlineType::HeavyHeavy,
            BarlineType::Tick,
            BarlineType::Short,
            BarlineType::NoBarline,
        ];
        let mut score = score_with_measures(kinds.len() as u32);
        for (measure, kind) in score.parts[0].measures.iter_mut().zip(kinds) {
            measure.barline.kind = kind;
        }
        let reread = round_trip(&score);
        let read_kinds: Vec<_> = reread.parts[0]
            .measures
            .iter()
            .map(|m| m.barline.kind)
            .collect();
        assert_eq!(read_kinds, kinds);
    }

    #[test]
    fn navigation_marks_round_trip() {
        let marks = [
            Navigation::Segno,
            Navigation::Coda,
            Navigation::DaCapo,
            Navigation::DaCapoAlFine,
            Navigation::DaCapoAlCoda,
            Navigation::DalSegno,
            Navigation::DalSegnoAlFine,
            Navigation::DalSegnoAlCoda,
            Navigation::Fine,
            Navigation::ToCoda,
        ];
        let mut score = score_with_measures(marks.len() as u32);
        for (measure, mark) in score.parts[0].measures.iter_mut().zip(marks) {
            measure.navigation = vec![mark];
        }
        let reread = round_trip(&score);
        let read_marks: Vec<_> = reread.parts[0]
            .measures
            .iter()
            .map(|m| m.navigation.clone())
            .collect();
        assert_eq!(read_marks, marks.map(|mark| vec![mark]));
    }

    #[test]
    fn several_navigation_marks_in_one_measure_round_trip() {
        let mut score = score_with_measures(2);
        score.parts[0].measures[0].navigation = vec![Navigation::Segno, Navigation::ToCoda];
        score.parts[0].measures[1].navigation = vec![Navigation::Coda, Navigation::Fine];
        let reread = round_trip(&score);
        let read_marks: Vec<_> = reread.parts[0]
            .measures
            .iter()
            .map(|m| m.navigation.clone())
            .collect();
        assert_eq!(
            read_marks,
            vec![
                vec![Navigation::Segno, Navigation::ToCoda],
                vec![Navigation::Coda, Navigation::Fine],
            ]
        );
    }

    const MUSESCORE_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<museScore version="4.20">
  <Score>
    <Division>480</Division>
    <metaTag name="workTitle">Fixture</metaTag>
    <Part id="1">
      <Staff id="1"><StaffType group="pitched"><name>stdNormal</name></StaffType></Staff>
      <trackName>Piano</trackName>
      <Instrument id="piano"><longName>Piano</longName></Instrument>
    </Part>
    <Staff id="1">
      <Measure>
        <Marker><style>repeat_left</style><text><sym>segno</sym></text><label>segno</label></Marker>
        <voice>
          <TimeSig><sigN>4</sigN><sigD>4</sigD></TimeSig>
          <Harmony><root>14</root><name>m7</name><base>11</base></Harmony>
          <Dynamic><subtype>pp</subtype><velocity>33</velocity></Dynamic>
          <Spanner type="HairPin">
            <HairPin><subtype>0</subtype></HairPin>
            <next><location><measures>1</measures></location></next>
          </Spanner>
          <Chord><acciaccatura/><durationType>eighth</durationType><Note><pitch>62</pitch><tpc>16</tpc></Note></Chord>
          <Chord>
            <durationType>half</durationType>
            <Articulation><subtype>articAccentStaccatoBelow</subtype></Articulation>
            <Articulation><subtype>ornamentMordentInverted</subtype></Articulation>
            <Note><pitch>60</pitch><tpc>14</tpc></Note>
          </Chord>
          <Breath><symbol>caesura</symbol></Breath>
          <Fermata><subtype>fermataAbove</subtype></Fermata>
          <Rest><durationType>half</durationType></Rest>
          <BarLine><subtype>double</subtype></BarLine>
        </voice>
      </Measure>
      <Measure>
        <voice>
          <Spanner type="HairPin"><prev><location><measures>-1</measures></location></prev></Spanner>
          <Spanner type="Volta">
            <Volta><endHookType>1</endHookType><beginText>1.</beginText><endings>1, 2</endings></Volta>
            <next><location><measures>1</measures></location></next>
          </Spanner>
          <Chord><durationType>whole</durationType><Ornament><subtype>ornamentTurn</subtype></Ornament><Note><pitch>64</pitch><tpc>18</tpc></Note></Chord>
        </voice>
      </Measure>
      <Measure>
        <Jump><text>D.S. al Fine</text><jumpTo>segno</jumpTo><playUntil>fine</playUntil><continueAt></continueAt></Jump>
        <voice>
          <Spanner type="Volta"><prev><location><measures>-1</measures></location></prev></Spanner>
          <Rest><durationType>measure</durationType></Rest>
        </voice>
      </Measure>
    </Staff>
  </Score>
</museScore>
"#;

    #[test]
    fn reads_musescore_markup() {
        let score = MuseScoreFormat
            .read(&mscz_from_mscx(MUSESCORE_FIXTURE))
            .unwrap();
        let measures = &score.parts[0].measures;
        assert_eq!(measures.len(), 3);
        assert_eq!(measures[0].navigation, vec![Navigation::Segno]);
        assert_eq!(measures[2].navigation, vec![Navigation::DalSegnoAlFine]);
        assert_eq!(measures[0].barline.kind, BarlineType::LightLight);
        assert_eq!(measures[1].barline.kind, BarlineType::Regular);
        // The volta's end element opens bar 3, so it covers bar 2 only.
        assert_eq!(
            measures[1].barline.ending,
            Some(Ending {
                numbers: vec![1, 2],
                measures: 1,
                closed: true,
            })
        );
        assert_eq!(measures[2].barline.ending, None);

        let events = &measures[0].voices[0].events;
        assert_eq!(events.len(), 3);
        let Event::Note(grace) = &events[0] else {
            panic!("expected the grace note")
        };
        assert!(grace.grace);
        assert_eq!(grace.dynamic, Some(Dynamic::Pp));
        assert_eq!(grace.wedge, Some(Wedge::CrescendoStart));
        assert_eq!(
            grace.chord_symbol,
            Some(ChordSymbol {
                root: ChordRoot::new(Step::C, 0),
                kind: ChordKind::MinorSeventh,
                bass: Some(ChordRoot::new(Step::E, -1)),
            })
        );

        let Event::Note(main) = &events[1] else {
            panic!("expected a note")
        };
        assert!(!main.grace);
        assert_eq!(
            main.articulations,
            vec![
                Articulation::Accent,
                Articulation::Staccato,
                Articulation::Caesura
            ]
        );
        assert_eq!(main.ornaments, vec![Ornament::InvertedMordent]);
        assert!(!main.fermata);

        let Event::Rest(rest) = &events[2] else {
            panic!("expected a rest")
        };
        assert!(rest.fermata);

        // The hairpin's end element opens bar 2, so it ends on bar 1's final
        // rest - which the model can't carry a wedge on - not on bar 2.
        let Event::Note(turn) = &measures[1].voices[0].events[0] else {
            panic!("expected a note")
        };
        assert_eq!(turn.ornaments, vec![Ornament::Turn]);
        assert_eq!(turn.wedge, None);
    }
}
