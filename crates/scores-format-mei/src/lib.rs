//! MEI (Music Encoding Initiative) plugin.
//!
//! There is no mature MEI crate on crates.io, so unlike the MusicXML and ABC
//! plugins this one talks XML directly via [`quick_xml`]. It reads and
//! writes a deliberately small, self-consistent *subset* of real MEI
//! (`<mei><music><body><mdiv><score>...`, `pname`/`oct`/`dur`/`dots`/
//! `accid.ges`/`tie` attributes, `<tuplet num="" numbase="">` containers,
//! etc.) rather than the full ODD-defined schema - enough to round-trip a
//! [`scores_core::Score`], not to validate against arbitrary MEI files.
//!
//! Multiple simultaneous voices are separate `<layer n="...">` elements
//! within a `<staff>` - each layer's own content is already sequential and
//! independent, so (unlike MusicXML) no `<backup>`-style rewind is needed.
//!
//! Notations, as written:
//! - Grace notes: `@grace="unacc"` (any `@grace` value reads as grace, also
//!   on a `<chord>`).
//! - Chord symbols: `<harm>` holding the [`scores_core::ChordSymbol`] text
//!   form (`Cmaj7`, `F#m7b5/E`, `N.C.`); text that doesn't parse as a chord
//!   symbol is ignored on read.
//! - Dynamics/wedges: `<dynam>`/`<hairpin form="cres|dim">` (`form="end"`
//!   for a wedge stop is this plugin's own extension).
//! - Chord symbols, dynamics and wedges are written as siblings immediately
//!   before the note/chord/rest they apply to, rather than as real MEI
//!   control events after the staves linked by `@startid`.
//! - Articulations: `<artic artic="...">` children of `<note>`, using MEI's
//!   `data.ARTICULATION` codes (breath mark/caesura use this plugin's own
//!   `breath`/`caes`, since real MEI has separate elements for those).
//! - Ornaments: `<trill>`, `<mordent form="lower|upper">`, `<turn
//!   form="lower|upper">`, or this plugin's own generic `<ornam type="...">`
//!   for the rest (tremolo with `count`), nested inside `<note>` - not valid
//!   real MEI, where these are `@startid` control events (and a tremolo is a
//!   `<bTrem>` container).
//! - Fermatas: `@fermata="above"` on `<note>`/`<rest>`.
//!
//! On read, real MEI control events (`<harm>`, `<dynam>`, `<hairpin>`,
//! `<fermata>`, `<trill>`, `<mordent>`, `<turn>`, `<ornam>`) placed after the
//! staves are also understood: they're resolved against the measure's
//! `xml:id`s via `@startid` (and `@endid` for a hairpin's stop), or via
//! `@tstamp`/`@staff` (the event starting at that beat, else the latest one
//! before it). A hairpin crossing a barline (`@tstamp2="1m+..."`) only gets
//! its start. `@artic`/`@fermata` attributes, `<artic>`/`@fermata` on a
//! `<chord>` (applied to its first note) and markings nested inside
//! `<note>`/`<rest>` all read too.
//!
//! Measure-level marks:
//! - Repeats use `@left="rptstart"`/`@right="rptend"` on `<measure>` (with a
//!   non-standard `@repeat.count` when not 2); `rptboth` reads as both.
//!   A repeat barline's visual style is implied by `rptend`, so a
//!   non-regular [`scores_core::BarlineType`] on the same measure is lost.
//! - Barline styles are `@right` (`dbl`, `end`, `heavy`, `dblheavy`,
//!   `dotted`, `dashed`, `invis`); MEI has no heavy-light, tick or short
//!   barline, so those use this plugin's own `heavyLight`/`tick`/`short`.
//! - Endings are real `<ending n="1">` containers wrapping every measure
//!   the bracket spans, with `@lendsym="angledown"` for a closed bracket
//!   and `"none"` for an open one. `<ending>`s can't nest, so one is cut
//!   short where the next ending starts.
//! - D.C./D.S./Coda/Segno/Fine/To Coda are MEI 5 `<repeatMark func="...">`
//!   on staff 1 (text such as "D.C. al Fine" distinguishes the variants),
//!   one per navigation mark of the measure.
//!   On read, a `<dir>` with recognizable text ("D.S. al Coda", "Fine", ...)
//!   and older versions' invented `<navigation type="...">` also work.
//!
//! Known simplifications, beyond the ones shared with the other plugins:
//! - `<meiHead>` is reduced to `<title>`/`<composer>` under `<titleStmt>`
//!   (`<composer>` as a direct child is not valid full MEI).
//! - Part names and initial clef/key/meter are stored as plain attributes
//!   on `<staffDef>` (real MEI uses a `<label>` child element and the
//!   values are otherwise the same).
//! - Mid-score clef/key/meter changes are written as bare `<clef>`,
//!   `<keySig>`, `<meterSig>` elements at the start of the first layer,
//!   rather than a nested `<scoreDef>`.
//! - Tempo is not carried (no `<tempo>` support).

mod convert;
mod read;
mod write;

use scores_core::{Format, FormatReader, FormatWriter, Result, Score};

pub struct MeiFormat;

impl FormatReader for MeiFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for MeiFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for MeiFormat {
    fn id(&self) -> &'static str {
        "mei"
    }

    fn name(&self) -> &'static str {
        "MEI"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["mei"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::{
        Articulation, BarlineType, ChordKind, ChordRoot, ChordSymbol, Clef, Duration, Dynamic,
        Ending, Event, KeySignature, Measure, MeasureAttributes, Mode, Navigation, Note, NoteValue,
        Ornament, Part, Pitch, Rest, Score, Step, Tie, TimeSignature, Tuplet, Voice, Wedge,
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
                            fifths: 2,
                            mode: Mode::Major,
                        }),
                        clef: Some(Clef::Treble),
                        tempo: None,
                    },
                    events,
                )],
            }],
        }
    }

    #[test]
    fn writes_well_formed_xml() {
        let bytes = MeiFormat.write(&scale_score()).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("<mei"));
        assert!(text.contains("<title>Scale</title>"));
        assert!(text.contains("key.sig=\"2s\""));
    }

    #[test]
    fn round_trips_pitches_durations_and_metadata() {
        let original = scale_score();
        let bytes = MeiFormat.write(&original).unwrap();
        let reread = MeiFormat.read(&bytes).unwrap();

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

        for (a, b) in original.parts[0].measures[0].voices[0]
            .events
            .iter()
            .zip(&first_measure.voices[0].events)
        {
            let (Event::Note(a), Event::Note(b)) = (a, b) else {
                panic!()
            };
            assert_eq!(a.pitch, b.pitch);
            assert_eq!(a.duration.quarter_length(), b.duration.quarter_length());
        }
    }

    #[test]
    fn round_trips_chords_ties_and_tuplets() {
        let mut score = scale_score();
        score.parts[0].measures[0].voices[0].events = vec![
            Event::Chord(vec![
                Note::new(
                    Pitch::new(Step::C, 0, 4),
                    Duration::new(NoteValue::Half),
                    Tie::None,
                ),
                Note::new(
                    Pitch::new(Step::E, 0, 4),
                    Duration::new(NoteValue::Half),
                    Tie::None,
                ),
            ]),
            Event::Note(Note::new(
                Pitch::new(Step::G, 0, 4),
                Duration {
                    base: NoteValue::Eighth,
                    dots: 0,
                    tuplet: Some(Tuplet {
                        actual: 3,
                        normal: 2,
                    }),
                },
                Tie::Start,
            )),
            Event::Note(Note::new(
                Pitch::new(Step::G, 0, 4),
                Duration {
                    base: NoteValue::Eighth,
                    dots: 0,
                    tuplet: Some(Tuplet {
                        actual: 3,
                        normal: 2,
                    }),
                },
                Tie::Stop,
            )),
        ];

        let bytes = MeiFormat.write(&score).unwrap();
        let reread = MeiFormat.read(&bytes).unwrap();
        let events = &reread.parts[0].measures[0].voices[0].events;

        let Event::Chord(notes) = &events[0] else {
            panic!("expected a chord")
        };
        assert_eq!(notes.len(), 2);

        let Event::Note(tied_start) = &events[1] else {
            panic!()
        };
        assert_eq!(tied_start.tie, Tie::Start);
        assert_eq!(
            tied_start.duration.tuplet,
            Some(Tuplet {
                actual: 3,
                normal: 2
            })
        );
        let Event::Note(tied_stop) = &events[2] else {
            panic!()
        };
        assert_eq!(tied_stop.tie, Tie::Stop);
    }

    #[test]
    fn multiple_voices_round_trip_independently() {
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

        let bytes = MeiFormat.write(&score).unwrap();
        let reread = MeiFormat.read(&bytes).unwrap();
        let voices = &reread.parts[0].measures[0].voices;
        assert_eq!(voices.len(), 2);

        let voice1 = voices.iter().find(|v| v.number == 1).unwrap();
        assert_eq!(voice1.events.len(), 4);
        let voice2 = voices.iter().find(|v| v.number == 2).unwrap();
        let Event::Note(bass) = &voice2.events[0] else {
            panic!()
        };
        assert_eq!(bass.pitch.step, Step::C);
        assert_eq!(bass.pitch.octave, 3);
    }

    #[test]
    fn grace_note_round_trips() {
        let mut score = scale_score();
        let mut grace = Note::new(
            Pitch::new(Step::B, 0, 4),
            Duration::new(NoteValue::Eighth),
            Tie::None,
        );
        grace.grace = true;
        score.parts[0].measures[0].voices[0]
            .events
            .insert(0, Event::Note(grace));

        let bytes = MeiFormat.write(&score).unwrap();
        let reread = MeiFormat.read(&bytes).unwrap();
        let events = &reread.parts[0].measures[0].voices[0].events;

        assert_eq!(events.len(), 5);
        let Event::Note(first) = &events[0] else {
            panic!()
        };
        assert!(first.grace);
        assert_eq!(first.pitch.step, Step::B);
    }

    #[test]
    fn dynamics_and_wedge_round_trip() {
        let mut score = scale_score();
        {
            let Event::Note(note) = &mut score.parts[0].measures[0].voices[0].events[0] else {
                panic!()
            };
            note.dynamic = Some(Dynamic::Mf);
            note.wedge = Some(Wedge::CrescendoStart);
        }
        {
            let Event::Note(note) = &mut score.parts[0].measures[0].voices[0].events[3] else {
                panic!()
            };
            note.wedge = Some(Wedge::Stop);
        }

        let bytes = MeiFormat.write(&score).unwrap();
        let reread = MeiFormat.read(&bytes).unwrap();
        let events = &reread.parts[0].measures[0].voices[0].events;

        let Event::Note(first) = &events[0] else {
            panic!()
        };
        assert_eq!(first.dynamic, Some(Dynamic::Mf));
        assert_eq!(first.wedge, Some(Wedge::CrescendoStart));
        let Event::Note(last) = &events[3] else {
            panic!()
        };
        assert_eq!(last.wedge, Some(Wedge::Stop));
    }

    #[test]
    fn articulations_ornaments_and_fermata_round_trip() {
        let mut score = scale_score();
        let Event::Note(note) = &mut score.parts[0].measures[0].voices[0].events[0] else {
            panic!()
        };
        note.articulations.push(Articulation::Accent);
        note.articulations.push(Articulation::Staccato);
        note.ornaments.push(Ornament::TrillMark);
        note.ornaments.push(Ornament::Tremolo(3));
        note.fermata = true;

        let bytes = MeiFormat.write(&score).unwrap();
        let reread = MeiFormat.read(&bytes).unwrap();
        let Event::Note(reread_note) = &reread.parts[0].measures[0].voices[0].events[0] else {
            panic!()
        };

        assert!(reread_note.articulations.contains(&Articulation::Accent));
        assert!(reread_note.articulations.contains(&Articulation::Staccato));
        assert!(reread_note.ornaments.contains(&Ornament::TrillMark));
        assert!(reread_note.ornaments.contains(&Ornament::Tremolo(3)));
        assert!(reread_note.fermata);
    }

    #[test]
    fn repeats_endings_and_navigation_round_trip() {
        let mut score = scale_score();
        score.parts[0].measures[0].barline.repeat_start = true;
        score.parts[0].measures.push(Measure::single_voice(
            2,
            MeasureAttributes::default(),
            vec![Event::Rest(Rest::new(Duration::new(NoteValue::Whole)))],
        ));
        score.parts[0].measures[1].barline.repeat_end = Some(3);
        score.parts[0].measures[1].barline.ending = Some(Ending {
            numbers: vec![1],
            measures: 1,
            closed: true,
        });
        score.parts[0].measures[1].navigation = vec![Navigation::DalSegnoAlFine];

        let bytes = MeiFormat.write(&score).unwrap();
        let reread = MeiFormat.read(&bytes).unwrap();
        let measures = &reread.parts[0].measures;

        assert!(measures[0].barline.repeat_start);
        assert_eq!(measures[1].barline.repeat_end, Some(3));
        assert_eq!(
            measures[1].barline.ending,
            Some(Ending {
                numbers: vec![1],
                measures: 1,
                closed: true,
            })
        );
        assert_eq!(measures[1].navigation, vec![Navigation::DalSegnoAlFine]);
    }

    fn quarter(step: Step) -> Note {
        Note::new(
            Pitch::new(step, 0, 4),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        )
    }

    fn round_trip(score: &Score) -> Score {
        let bytes = MeiFormat.write(score).unwrap();
        MeiFormat.read(&bytes).unwrap()
    }

    /// Wraps `<measure>`/`<ending>` content in a minimal real-MEI document
    /// with one 4/4 treble staff.
    fn mei_doc(section: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<mei xmlns="http://www.music-encoding.org/ns/mei" meiversion="5.0">
  <music><body><mdiv><score>
    <scoreDef><staffGrp>
      <staffDef n="1" lines="5" clef.shape="G" clef.line="2" meter.count="4" meter.unit="4"/>
    </staffGrp></scoreDef>
    <section>{section}</section>
  </score></mdiv></body></music>
</mei>"#
        )
        .into_bytes()
    }

    #[test]
    fn chord_symbols_round_trip_on_notes_chords_and_rests() {
        let mut score = scale_score();
        let cmaj7 = ChordSymbol::parse("Cmaj7").unwrap();
        let slash = ChordSymbol::parse("F#m7b5/E").unwrap();
        let mut chord_note = quarter(Step::C);
        chord_note.chord_symbol = Some(slash.clone());
        let mut rest = Rest::new(Duration::new(NoteValue::Quarter));
        rest.chord_symbol = Some(ChordSymbol::no_chord());
        let mut note = quarter(Step::D);
        note.chord_symbol = Some(cmaj7.clone());
        score.parts[0].measures[0].voices[0].events = vec![
            Event::Note(note),
            Event::Chord(vec![chord_note, quarter(Step::E)]),
            Event::Rest(rest),
            Event::Note(quarter(Step::F)),
        ];

        let bytes = MeiFormat.write(&score).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("<harm>Cmaj7</harm>"));
        let reread = MeiFormat.read(&bytes).unwrap();
        let events = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(events[0].chord_symbol(), Some(&cmaj7));
        assert_eq!(events[1].chord_symbol(), Some(&slash));
        assert_eq!(events[2].chord_symbol(), Some(&ChordSymbol::no_chord()));
        assert_eq!(events[3].chord_symbol(), None);
        assert_eq!(reread.parts[0].measures, score.parts[0].measures);
    }

    #[test]
    fn chord_level_dynamics_stay_on_the_chord() {
        let mut score = scale_score();
        let mut first = quarter(Step::C);
        first.dynamic = Some(Dynamic::Ff);
        first.wedge = Some(Wedge::DiminuendoStart);
        score.parts[0].measures[0].voices[0].events = vec![
            Event::Chord(vec![first, quarter(Step::E)]),
            Event::Note(quarter(Step::G)),
        ];

        let reread = round_trip(&score);
        let events = &reread.parts[0].measures[0].voices[0].events;
        let Event::Chord(notes) = &events[0] else {
            panic!()
        };
        assert_eq!(notes[0].dynamic, Some(Dynamic::Ff));
        assert_eq!(notes[0].wedge, Some(Wedge::DiminuendoStart));
        // Previously the chord's dynamic leaked onto the following note.
        let Event::Note(next) = &events[1] else {
            panic!()
        };
        assert_eq!(next.dynamic, None);
        assert_eq!(next.wedge, None);
    }

    #[test]
    fn every_articulation_ornament_and_fermata_round_trips() {
        let mut score = scale_score();
        let articulations = vec![
            Articulation::Accent,
            Articulation::StrongAccent,
            Articulation::Staccato,
            Articulation::Tenuto,
            Articulation::DetachedLegato,
            Articulation::Staccatissimo,
            Articulation::Spiccato,
            Articulation::Scoop,
            Articulation::Plop,
            Articulation::Doit,
            Articulation::Falloff,
            Articulation::BreathMark,
            Articulation::Caesura,
            Articulation::Stress,
            Articulation::Unstress,
            Articulation::SoftAccent,
            Articulation::Other("snap".into()),
        ];
        let ornaments = vec![
            Ornament::TrillMark,
            Ornament::Turn,
            Ornament::DelayedTurn,
            Ornament::InvertedTurn,
            Ornament::DelayedInvertedTurn,
            Ornament::VerticalTurn,
            Ornament::InvertedVerticalTurn,
            Ornament::Shake,
            Ornament::WavyLine,
            Ornament::Mordent,
            Ornament::InvertedMordent,
            Ornament::Schleifer,
            Ornament::Tremolo(2),
            Ornament::Haydn,
            Ornament::Other("bebung".into()),
        ];
        let mut note = quarter(Step::C);
        note.articulations = articulations;
        note.ornaments = ornaments;
        note.fermata = true;
        let mut chord_top = quarter(Step::G);
        chord_top.fermata = true;
        let mut rest = Rest::new(Duration::new(NoteValue::Quarter));
        rest.fermata = true;
        score.parts[0].measures[0].voices[0].events = vec![
            Event::Note(note),
            Event::Chord(vec![quarter(Step::E), chord_top]),
            Event::Rest(rest),
        ];

        let reread = round_trip(&score);
        assert_eq!(reread.parts[0].measures, score.parts[0].measures);
    }

    #[test]
    fn every_barline_style_round_trips() {
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
        let mut score = scale_score();
        let template = score.parts[0].measures[0].clone();
        score.parts[0].measures = kinds
            .iter()
            .enumerate()
            .map(|(i, &kind)| {
                let mut m = template.clone();
                m.number = i as u32 + 1;
                if i > 0 {
                    m.attributes = MeasureAttributes::default();
                }
                m.barline.kind = kind;
                m
            })
            .collect();

        let bytes = MeiFormat.write(&score).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(r#"right="end""#));
        assert!(text.contains(r#"right="dbl""#));
        let reread = MeiFormat.read(&bytes).unwrap();
        let reread_kinds: Vec<_> = reread.parts[0]
            .measures
            .iter()
            .map(|m| m.barline.kind)
            .collect();
        assert_eq!(reread_kinds, kinds);
    }

    #[test]
    fn multi_measure_endings_round_trip() {
        let mut score = scale_score();
        let template = score.parts[0].measures[0].clone();
        score.parts[0].measures = (1..=6)
            .map(|n| {
                let mut m = template.clone();
                m.number = n;
                if n > 1 {
                    m.attributes = MeasureAttributes::default();
                }
                m
            })
            .collect();
        let ending = |numbers: Vec<u8>, measures, closed| {
            Some(Ending {
                numbers,
                measures,
                closed,
            })
        };
        let measures = &mut score.parts[0].measures;
        measures[0].barline.repeat_start = true;
        // A two-measure first ending that closes, then a two-measure open
        // second ending, then a one-measure closed ending.
        measures[1].barline.ending = ending(vec![1], 2, true);
        measures[2].barline.repeat_end = Some(2);
        measures[3].barline.ending = ending(vec![2], 2, false);
        measures[5].barline.ending = ending(vec![3, 4], 1, true);

        let bytes = MeiFormat.write(&score).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("<ending").count(), 3);
        let reread = MeiFormat.read(&bytes).unwrap();
        assert_eq!(reread.parts[0].measures, score.parts[0].measures);
    }

    #[test]
    fn every_navigation_mark_round_trips() {
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
        let mut score = scale_score();
        let template = score.parts[0].measures[0].clone();
        score.parts[0].measures = marks
            .iter()
            .enumerate()
            .map(|(i, &mark)| {
                let mut m = template.clone();
                m.number = i as u32 + 1;
                if i > 0 {
                    m.attributes = MeasureAttributes::default();
                }
                m.navigation = vec![mark];
                m
            })
            .collect();

        let bytes = MeiFormat.write(&score).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(
            r#"<repeatMark func="daCapo" staff="1" tstamp="5">D.C. al Fine</repeatMark>"#
        ));
        let reread = MeiFormat.read(&bytes).unwrap();
        let reread_marks: Vec<_> = reread.parts[0]
            .measures
            .iter()
            .map(|m| m.navigation.clone())
            .collect();
        assert_eq!(reread_marks, marks.map(|mark| vec![mark]));
    }

    #[test]
    fn several_navigation_marks_in_one_measure_round_trip() {
        let mut score = scale_score();
        score.parts[0].measures[0].navigation = vec![Navigation::Segno, Navigation::ToCoda];
        let text = String::from_utf8(MeiFormat.write(&score).unwrap()).unwrap();
        assert_eq!(text.matches("<repeatMark").count(), 2);
        let reread = MeiFormat.read(text.as_bytes()).unwrap();
        assert_eq!(
            reread.parts[0].measures[0].navigation,
            vec![Navigation::Segno, Navigation::ToCoda]
        );
    }

    #[test]
    fn reads_real_mei_control_events_by_startid_and_tstamp() {
        let doc = mei_doc(
            r##"<measure n="1">
              <staff n="1"><layer n="1">
                <note xml:id="n1" pname="c" oct="4" dur="4"/>
                <chord xml:id="c1" dur="4" fermata="above">
                  <artic artic="acc stacc"/>
                  <note xml:id="c1a" pname="e" oct="4"/>
                  <note xml:id="c1b" pname="g" oct="4"/>
                </chord>
                <rest xml:id="r1" dur="4"/>
                <note xml:id="n2" pname="d" oct="4" dur="4" artic="ten"/>
              </layer></staff>
              <harm staff="1" startid="#n1">Dm7</harm>
              <harm staff="1" tstamp="3">G7</harm>
              <harm staff="1" tstamp="4.5">Cmaj7</harm>
              <dynam staff="1" startid="#c1">p</dynam>
              <hairpin staff="1" form="cres" startid="#n1" endid="#n2"/>
              <trill staff="1" startid="#c1b"/>
              <fermata staff="1" startid="#r1"/>
            </measure>"##,
        );
        let score = MeiFormat.read(&doc).unwrap();
        let events = &score.parts[0].measures[0].voices[0].events;

        let Event::Note(n1) = &events[0] else {
            panic!()
        };
        assert_eq!(n1.chord_symbol, ChordSymbol::parse("Dm7"));
        assert_eq!(n1.wedge, Some(Wedge::CrescendoStart));

        let Event::Chord(chord) = &events[1] else {
            panic!()
        };
        assert_eq!(chord[0].dynamic, Some(Dynamic::P));
        assert!(chord[0].fermata);
        assert_eq!(
            chord[0].articulations,
            vec![Articulation::Accent, Articulation::Staccato]
        );
        assert_eq!(chord[1].ornaments, vec![Ornament::TrillMark]);

        let Event::Rest(rest) = &events[2] else {
            panic!()
        };
        assert!(rest.fermata);
        assert_eq!(
            rest.chord_symbol,
            Some(ChordSymbol::new(
                ChordRoot::new(Step::G, 0),
                ChordKind::Dominant
            ))
        );

        // tstamp 4.5 has no onset of its own: it lands on the note at beat 4.
        let Event::Note(n2) = &events[3] else {
            panic!()
        };
        assert_eq!(n2.chord_symbol, ChordSymbol::parse("Cmaj7"));
        assert_eq!(n2.wedge, Some(Wedge::Stop));
        assert_eq!(n2.articulations, vec![Articulation::Tenuto]);
    }

    #[test]
    fn reads_real_mei_grace_chords_repeats_endings_and_navigation() {
        let layer =
            r#"<staff n="1"><layer n="1"><note pname="c" oct="4" dur="1"/></layer></staff>"#;
        let doc = mei_doc(&format!(
            r#"<measure n="1" right="rptboth">
                 <staff n="1"><layer n="1">
                   <chord dur="8" grace="acc"><note pname="d" oct="4"/><note pname="f" oct="4"/></chord>
                   <note pname="c" oct="4" dur="1"/>
                 </layer></staff>
                 <repeatMark func="segno" staff="1" tstamp="1"/>
               </measure>
               <ending n="1" lendsym="none">
                 <measure n="2">{layer}<dir staff="1" tstamp="5">D.S. al Coda</dir></measure>
                 <measure n="3" right="rptend">{layer}</measure>
                 <measure n="4">{layer}</measure>
               </ending>
               <measure n="5" left="rptboth" right="end">{layer}<repeatMark func="coda" staff="1" tstamp="1">To Coda</repeatMark></measure>
               <measure n="6" right="lightLight">{layer}<navigation type="fine"/></measure>"#
        ));
        let score = MeiFormat.read(&doc).unwrap();
        let measures = &score.parts[0].measures;

        let Event::Chord(grace) = &measures[0].voices[0].events[0] else {
            panic!()
        };
        assert!(grace.iter().all(|n| n.grace));
        assert_eq!(measures[0].barline.repeat_end, Some(2));
        assert_eq!(measures[0].navigation, vec![Navigation::Segno]);

        assert!(measures[1].barline.repeat_start);
        assert_eq!(measures[1].navigation, vec![Navigation::DalSegnoAlCoda]);
        assert_eq!(
            measures[1].barline.ending,
            Some(Ending {
                numbers: vec![1],
                measures: 3,
                closed: false,
            })
        );
        assert_eq!(measures[2].barline.ending, None);
        assert_eq!(measures[3].barline.ending, None);
        // left="rptboth" also closes the repeat at the previous measure.
        assert_eq!(measures[3].barline.repeat_end, Some(2));
        assert!(measures[4].barline.repeat_start);
        assert_eq!(measures[4].barline.kind, BarlineType::LightHeavy);
        assert_eq!(measures[4].navigation, vec![Navigation::ToCoda]);
        assert_eq!(measures[5].barline.kind, BarlineType::LightLight);
        assert_eq!(measures[5].navigation, vec![Navigation::Fine]);
    }

    #[test]
    fn reads_legacy_nested_markings_and_ignores_non_chord_harm_text() {
        let doc = mei_doc(
            r#"<measure n="1"><staff n="1"><layer n="1">
                 <harm>not a chord</harm>
                 <dynam>sfz</dynam>
                 <note pname="c" oct="4" dur="2"><artic artic="detls"/><fermata/><mordent form="upper"/></note>
                 <rest dur="2"><fermata/></rest>
               </layer></staff></measure>"#,
        );
        let score = MeiFormat.read(&doc).unwrap();
        let events = &score.parts[0].measures[0].voices[0].events;
        let Event::Note(note) = &events[0] else {
            panic!()
        };
        assert_eq!(note.chord_symbol, None);
        assert_eq!(note.dynamic, Some(Dynamic::Sfz));
        assert!(note.fermata);
        assert_eq!(note.articulations, vec![Articulation::DetachedLegato]);
        assert_eq!(note.ornaments, vec![Ornament::InvertedMordent]);
        let Event::Rest(rest) = &events[1] else {
            panic!()
        };
        assert!(rest.fermata);
    }
}
