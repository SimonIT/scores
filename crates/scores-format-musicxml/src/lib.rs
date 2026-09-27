//! MusicXML plugin, built on the [`musicxml`] crate (a complete
//! implementation of the MusicXML 4.0 element tree) for parsing/serializing
//! the XML itself; this module only maps between its `ScorePartwise` tree
//! and our own [`scores_core::Score`].
//!
//! Mapped in both directions:
//! - Multiple simultaneous voices, via `<backup>` between them.
//! - Grace notes via `<grace>` (no `<duration>` element, matching the spec -
//!   they borrow no real time); their `<type>`/`<dot>` give the notated value.
//! - Dynamics and wedges (hairpins) via `<direction><direction-type>`;
//!   dynamics written as a note `<notations><dynamics>` are read too.
//! - Articulations, ornaments and fermata (on notes and rests) via
//!   `<notations>`.
//! - Chord symbols via `<harmony>` (root, kind, bass) placed before the
//!   note/rest they sit over; every `kind-value` with a [`scores_core::ChordKind`]
//!   equivalent maps directly, "N.C." is kind `none`, and anything else is
//!   kind `other` with the suffix as its `text` attribute.
//! - Tempo via `<sound tempo="...">`.
//! - Barline style (`<bar-style>`), repeats (`<repeat>`) and 1st/2nd endings
//!   (`<ending>`, started on the left barline of the bracket's first measure
//!   and stopped - `stop` if closed, `discontinue` if open - on the right
//!   barline of its last; an unstopped bracket reads as open up to the end
//!   of the part).
//! - Segno/Coda/D.C./D.S./Fine/To Coda as a `<direction>` carrying both a
//!   visible mark (`<segno>`/`<coda>` or `<words>`) and a `<sound>` with
//!   `segno`/`coda`/`dacapo`/`dalsegno`/`fine`/`tocoda`; on reading, any one of
//!   these is enough (plain `<words>` such as "D.S. al Fine" are recognized).
//!   Each of a measure's navigation marks is its own `<direction>`.
//!
//! Not modeled:
//! - Only traditional (fifths-based) key signatures are supported;
//!   non-traditional ones default to C major.
//! - Only the first `<beats>`/`<beat-type>` pair of a (possibly composite)
//!   time signature is used.
//! - Cue notes are read as ordinary notes; unpitched percussion notes are
//!   read as rests of the same duration.
//! - Accidental *display* is not tracked separately from the pitch's
//!   `alter` semitone value.
//! - Articulation/ornament shapes and spans aren't tracked: breath marks and
//!   caesuras are written with the default glyph, a `WavyLine` as a trill
//!   line starting and stopping on its note (only a line's start is read),
//!   and `Tremolo` as a single-note tremolo.
//! - Grace-note slashes, dynamics before a rest (they move to the next
//!   note), and chord symbols over a grace note or a non-first chord note
//!   (they move to the next real event / the chord's first note) are lost.
//! - Harmony: only the first harmony-chord of a stacked `<harmony>` is used;
//!   Roman-numeral (`<numeral>`/`<function>`) harmonies, inversions and frame
//!   diagrams are ignored; `<degree>` alterations are read by folding them
//!   into the suffix text (`7` + flat 9 -> `7b9`) but never written.

mod read;
mod write;

use scores_core::{Format, FormatReader, FormatWriter, Result, Score};

pub struct MusicXmlFormat;

impl FormatReader for MusicXmlFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for MusicXmlFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for MusicXmlFormat {
    fn id(&self) -> &'static str {
        "musicxml"
    }

    fn name(&self) -> &'static str {
        "MusicXML"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["musicxml", "xml", "mxl"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::Duration;
    use scores_core::{
        Articulation, Barline, BarlineType, ChordKind, ChordRoot, ChordSymbol, Clef, Dynamic,
        Ending, Event, KeySignature, Measure, MeasureAttributes, Mode, Navigation, Note, NoteValue,
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
        let bytes = MusicXmlFormat.write(&scale_score()).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("<score-partwise"));
        assert!(text.contains("<movement-title>Scale</movement-title>"));
    }

    #[test]
    fn round_trips_pitches_durations_and_metadata() {
        let original = scale_score();
        let bytes = MusicXmlFormat.write(&original).unwrap();
        let reread = MusicXmlFormat.read(&bytes).unwrap();

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
                Duration::new(NoteValue::Quarter),
                Tie::Start,
            )),
            Event::Note(Note::new(
                Pitch::new(Step::G, 0, 4),
                Duration::new(NoteValue::Quarter),
                Tie::Stop,
            )),
        ];

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let reread = MusicXmlFormat.read(&bytes).unwrap();
        let events = &reread.parts[0].measures[0].voices[0].events;

        let Event::Chord(notes) = &events[0] else {
            panic!("expected a chord")
        };
        assert_eq!(notes.len(), 2);

        let Event::Note(tied_start) = &events[1] else {
            panic!()
        };
        assert_eq!(tied_start.tie, Tie::Start);
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

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let reread = MusicXmlFormat.read(&bytes).unwrap();
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
    fn grace_note_borrows_no_time() {
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

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let reread = MusicXmlFormat.read(&bytes).unwrap();
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

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let reread = MusicXmlFormat.read(&bytes).unwrap();
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
        note.fermata = true;

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let reread = MusicXmlFormat.read(&bytes).unwrap();
        let Event::Note(reread_note) = &reread.parts[0].measures[0].voices[0].events[0] else {
            panic!()
        };

        assert!(reread_note.articulations.contains(&Articulation::Accent));
        assert!(reread_note.articulations.contains(&Articulation::Staccato));
        assert!(reread_note.ornaments.contains(&Ornament::TrillMark));
        assert!(reread_note.fermata);
    }

    #[test]
    fn tempo_round_trips() {
        let mut score = scale_score();
        score.parts[0].measures[0].attributes.tempo = Some(Tempo { bpm: 96.0 });

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let reread = MusicXmlFormat.read(&bytes).unwrap();

        assert_eq!(
            reread.parts[0].measures[0].attributes.tempo,
            Some(Tempo { bpm: 96.0 })
        );
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
        score.parts[0].measures[1].barline.repeat_end = Some(2);
        score.parts[0].measures[1].barline.ending = Some(Ending {
            numbers: vec![1],
            measures: 1,
            closed: true,
        });
        score.parts[0].measures[1].navigation = vec![Navigation::DalSegnoAlFine];

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let reread = MusicXmlFormat.read(&bytes).unwrap();
        let measures = &reread.parts[0].measures;

        assert!(measures[0].barline.repeat_start);
        assert_eq!(measures[1].barline.repeat_end, Some(2));
        assert_eq!(
            measures[1].barline.ending,
            Some(Ending {
                numbers: vec![1],
                measures: 1,
                closed: true,
            })
        );
        assert_eq!(measures[1].navigation, vec![Navigation::DalSegnoAlFine]);
        let _: &Barline = &measures[0].barline;
    }

    fn quarter(step: Step) -> Note {
        Note::new(
            Pitch::new(step, 0, 4),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        )
    }

    fn round_trip(score: &Score) -> Score {
        let bytes = MusicXmlFormat.write(score).unwrap();
        MusicXmlFormat.read(&bytes).unwrap()
    }

    /// Wraps the given `<measure>` elements into a minimal one-part document
    /// (4/4, one division per quarter note).
    fn read_measures(measures: &str) -> Score {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<score-partwise version="4.0">
  <part-list><score-part id="P1"><part-name>Music</part-name></score-part></part-list>
  <part id="P1">{measures}</part>
</score-partwise>"#
        );
        MusicXmlFormat.read(xml.as_bytes()).unwrap()
    }

    const ATTRIBUTES: &str = "<attributes><divisions>1</divisions><time><beats>4</beats><beat-type>4</beat-type></time></attributes>";

    fn xml_quarter(step: &str) -> String {
        format!(
            "<note><pitch><step>{step}</step><octave>4</octave></pitch><duration>1</duration><voice>1</voice><type>quarter</type></note>"
        )
    }

    fn all_chord_kinds() -> Vec<ChordKind> {
        vec![
            ChordKind::Major,
            ChordKind::Minor,
            ChordKind::Augmented,
            ChordKind::Diminished,
            ChordKind::Dominant,
            ChordKind::MajorSeventh,
            ChordKind::MinorSeventh,
            ChordKind::DiminishedSeventh,
            ChordKind::AugmentedSeventh,
            ChordKind::HalfDiminished,
            ChordKind::MajorMinor,
            ChordKind::MajorSixth,
            ChordKind::MinorSixth,
            ChordKind::DominantNinth,
            ChordKind::MajorNinth,
            ChordKind::MinorNinth,
            ChordKind::Dominant11th,
            ChordKind::Major11th,
            ChordKind::Minor11th,
            ChordKind::Dominant13th,
            ChordKind::Major13th,
            ChordKind::Minor13th,
            ChordKind::SuspendedSecond,
            ChordKind::SuspendedFourth,
            ChordKind::Power,
            ChordKind::Other("7b9".to_string()),
        ]
    }

    #[test]
    fn every_chord_kind_round_trips() {
        let mut score = scale_score();
        let measure = score.parts[0].measures[0].clone();
        score.parts[0].measures = all_chord_kinds()
            .into_iter()
            .enumerate()
            .map(|(i, kind)| {
                let mut m = measure.clone();
                m.number = i as u32 + 1;
                if i > 0 {
                    m.attributes = MeasureAttributes::default();
                }
                m.voices[0].events[0].set_chord_symbol(Some(ChordSymbol {
                    root: ChordRoot::new(Step::B, -1),
                    kind,
                    bass: Some(ChordRoot::new(Step::F, 1)),
                }));
                m.voices[0].events[2].set_chord_symbol(Some(ChordSymbol::new(
                    ChordRoot::new(Step::G, 0),
                    ChordKind::Major,
                )));
                m
            })
            .collect();

        assert_eq!(round_trip(&score), score);
    }

    #[test]
    fn chord_symbols_on_rests_chords_and_no_chord_round_trip() {
        let mut score = scale_score();
        let mut rest = Rest::new(Duration::new(NoteValue::Quarter));
        rest.chord_symbol = Some(ChordSymbol::no_chord());
        let mut chord = Event::Chord(vec![quarter(Step::C), quarter(Step::E)]);
        chord.set_chord_symbol(ChordSymbol::parse("Am7/C"));
        let events = &mut score.parts[0].measures[0].voices[0].events;
        events[0] = Event::Rest(rest);
        events[1] = chord;

        let reread = round_trip(&score);
        assert_eq!(reread, score);
        let events = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(events[1].chord_symbol().unwrap().to_string(), "Am7/C");
    }

    #[test]
    fn reads_harmony_elements() {
        let measure = format!(
            r#"<measure number="1">{ATTRIBUTES}
  <harmony><root><root-step>G</root-step></root><kind>dominant</kind>
    <degree><degree-value>9</degree-value><degree-alter>-1</degree-alter><degree-type>add</degree-type></degree></harmony>
  {n1}
  <harmony><root><root-step>E</root-step><root-alter>-1</root-alter></root><kind text="+">augmented</kind>
    <bass><bass-step>G</bass-step></bass></harmony>
  {n2}
  <harmony><kind>none</kind></harmony>
  {n3}
  <harmony><root><root-step>D</root-step></root><kind text="7alt">other</kind></harmony>
  <note><rest/><duration>1</duration><voice>1</voice><type>quarter</type></note>
</measure>"#,
            n1 = xml_quarter("C"),
            n2 = xml_quarter("D"),
            n3 = xml_quarter("E"),
        );
        let score = read_measures(&measure);
        let symbols: Vec<String> = score.parts[0].measures[0].voices[0]
            .events
            .iter()
            .map(|e| e.chord_symbol().unwrap().to_string())
            .collect();
        assert_eq!(symbols, ["G7b9", "Ebaug/G", "N.C.", "D7alt"]);
        let first = score.parts[0].measures[0].voices[0].events[0].chord_symbol();
        assert_eq!(first.unwrap().kind, ChordKind::Other("7b9".to_string()));
    }

    #[test]
    fn harmony_attaches_to_the_following_voice_after_backup() {
        let measure = format!(
            r#"<measure number="1">{ATTRIBUTES}
  <note><pitch><step>C</step><octave>5</octave></pitch><duration>4</duration><voice>1</voice><type>whole</type></note>
  <backup><duration>4</duration></backup>
  <harmony><root><root-step>F</root-step></root><kind>major</kind></harmony>
  <note><pitch><step>F</step><octave>3</octave></pitch><duration>4</duration><voice>2</voice><type>whole</type></note>
</measure>"#
        );
        let score = read_measures(&measure);
        let voices = &score.parts[0].measures[0].voices;
        assert_eq!(voices[0].events[0].chord_symbol(), None);
        assert_eq!(voices[1].events[0].chord_symbol().unwrap().to_string(), "F");
    }

    #[test]
    fn rest_fermata_and_remaining_marks_round_trip() {
        let mut score = scale_score();
        let mut rest = Rest::new(Duration::new(NoteValue::Quarter));
        rest.fermata = true;
        let mut note = quarter(Step::G);
        note.articulations = vec![Articulation::BreathMark, Articulation::Caesura];
        note.ornaments = vec![Ornament::WavyLine, Ornament::Tremolo(3), Ornament::Mordent];
        let mut grace = Note::new(
            Pitch::new(Step::A, -1, 4),
            Duration::dotted(NoteValue::Sixteenth, 1),
            Tie::None,
        );
        grace.grace = true;
        grace.articulations = vec![Articulation::Accent];
        let events = &mut score.parts[0].measures[0].voices[0].events;
        events[0] = Event::Rest(rest);
        events[1] = Event::Note(note);
        events.insert(2, Event::Note(grace));

        assert_eq!(round_trip(&score), score);
    }

    #[test]
    fn every_dynamic_and_wedge_round_trips() {
        let dynamics = [
            Dynamic::P,
            Dynamic::Pp,
            Dynamic::Ppp,
            Dynamic::Pppp,
            Dynamic::Ppppp,
            Dynamic::Pppppp,
            Dynamic::F,
            Dynamic::Ff,
            Dynamic::Fff,
            Dynamic::Ffff,
            Dynamic::Fffff,
            Dynamic::Ffffff,
            Dynamic::Mp,
            Dynamic::Mf,
            Dynamic::Sf,
            Dynamic::Sfp,
            Dynamic::Sfpp,
            Dynamic::Fp,
            Dynamic::Rf,
            Dynamic::Rfz,
            Dynamic::Sfz,
            Dynamic::Sffz,
            Dynamic::Fz,
            Dynamic::N,
            Dynamic::Pf,
            Dynamic::Sfzp,
            Dynamic::Other("sfffz".to_string()),
        ];
        let wedges = [Wedge::CrescendoStart, Wedge::Stop, Wedge::DiminuendoStart];
        let mut score = scale_score();
        score.parts[0].measures[0].voices[0].events = dynamics
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let mut n = quarter(Step::C);
                n.dynamic = Some(d.clone());
                n.wedge = Some(wedges[i % 3]);
                Event::Note(n)
            })
            .collect();

        assert_eq!(round_trip(&score), score);
    }

    #[test]
    fn reads_notation_dynamics_and_trailing_wedge_stop() {
        let measure = format!(
            r#"<measure number="1">{ATTRIBUTES}
  <direction><direction-type><wedge type="diminuendo"/></direction-type></direction>
  <note><pitch><step>C</step><octave>4</octave></pitch><duration>2</duration><voice>1</voice><type>half</type>
    <notations><dynamics><sfz/></dynamics></notations></note>
  {n}{n}
  <direction><direction-type><wedge type="stop"/></direction-type></direction>
</measure>"#,
            n = xml_quarter("D"),
        );
        let score = read_measures(&measure);
        let events = &score.parts[0].measures[0].voices[0].events;
        let (Event::Note(first), Event::Note(last)) = (&events[0], &events[2]) else {
            panic!()
        };
        assert_eq!(first.dynamic, Some(Dynamic::Sfz));
        assert_eq!(first.wedge, Some(Wedge::DiminuendoStart));
        assert_eq!(last.wedge, Some(Wedge::Stop));
    }

    #[test]
    fn reads_grace_note_type_and_rest_fermata() {
        let measure = format!(
            r#"<measure number="1">{ATTRIBUTES}
  <note><grace slash="yes"/><pitch><step>B</step><octave>4</octave></pitch><voice>1</voice><type>16th</type></note>
  <note><pitch><step>C</step><octave>5</octave></pitch><duration>3</duration><voice>1</voice><type>half</type><dot/></note>
  <note><rest/><duration>1</duration><voice>1</voice><type>quarter</type><notations><fermata/></notations></note>
</measure>"#
        );
        let score = read_measures(&measure);
        let events = &score.parts[0].measures[0].voices[0].events;
        assert_eq!(events.len(), 3);
        let Event::Note(grace) = &events[0] else {
            panic!()
        };
        assert!(grace.grace);
        assert_eq!(grace.duration, Duration::new(NoteValue::Sixteenth));
        let Event::Rest(rest) = &events[2] else {
            panic!()
        };
        assert!(rest.fermata);
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
        let measure = score.parts[0].measures[0].clone();
        score.parts[0].measures = kinds
            .iter()
            .enumerate()
            .map(|(i, &kind)| {
                let mut m = measure.clone();
                m.number = i as u32 + 1;
                if i > 0 {
                    m.attributes = MeasureAttributes::default();
                }
                m.barline.kind = kind;
                m.barline.repeat_start = i == 2;
                m
            })
            .collect();

        assert_eq!(round_trip(&score), score);
    }

    #[test]
    fn left_barline_style_does_not_set_measure_barline() {
        let measure = format!(
            r#"<measure number="1">
  <barline location="left"><bar-style>heavy-light</bar-style><repeat direction="forward"/></barline>
  {ATTRIBUTES}
  <note><rest/><duration>4</duration><voice>1</voice><type>whole</type></note>
  <barline location="right"><bar-style>light-heavy</bar-style><repeat direction="backward"/></barline>
</measure>"#
        );
        let score = read_measures(&measure);
        let barline = &score.parts[0].measures[0].barline;
        assert!(barline.repeat_start);
        assert_eq!(barline.repeat_end, Some(2));
        assert_eq!(barline.kind, BarlineType::LightHeavy);
    }

    #[test]
    fn volta_brackets_round_trip_and_are_closed() {
        let mut score = scale_score();
        let measure = score.parts[0].measures[0].clone();
        score.parts[0].measures = (1..=5)
            .map(|n| {
                let mut m = measure.clone();
                m.number = n;
                if n > 1 {
                    m.attributes = MeasureAttributes::default();
                }
                m
            })
            .collect();
        let measures = &mut score.parts[0].measures;
        measures[0].barline.repeat_start = true;
        // A closed two-measure 1st ending, then an open two-measure 2nd
        // ending.
        measures[1].barline.ending = Some(Ending {
            numbers: vec![1, 2],
            measures: 2,
            closed: true,
        });
        measures[2].barline.repeat_end = Some(3);
        measures[3].barline.ending = Some(Ending {
            numbers: vec![3],
            measures: 2,
            closed: false,
        });
        measures[4].barline.kind = BarlineType::LightHeavy;

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(r#"<ending number="1,2" type="start">1, 2.</ending>"#));
        assert!(text.contains(r#"<ending number="1,2" type="stop"/>"#));
        assert!(text.contains(r#"<ending number="3" type="start">3.</ending>"#));
        assert!(text.contains(r#"<ending number="3" type="discontinue"/>"#));
        assert_eq!(MusicXmlFormat.read(&bytes).unwrap(), score);
    }

    #[test]
    fn one_measure_closed_and_open_endings_round_trip() {
        let mut score = scale_score();
        let measure = score.parts[0].measures[0].clone();
        score.parts[0].measures = (1..=3)
            .map(|n| {
                let mut m = measure.clone();
                m.number = n;
                if n > 1 {
                    m.attributes = MeasureAttributes::default();
                }
                m
            })
            .collect();
        let measures = &mut score.parts[0].measures;
        // A closed one-measure 1st ending without a repeat (not inferable
        // from the repeat any more), then an open one-measure 2nd ending.
        measures[1].barline.ending = Some(Ending {
            numbers: vec![1],
            measures: 1,
            closed: true,
        });
        measures[2].barline.ending = Some(Ending {
            numbers: vec![2],
            measures: 1,
            closed: false,
        });

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(r#"<ending number="1" type="stop"/>"#));
        assert!(text.contains(r#"<ending number="2" type="discontinue"/>"#));
        assert_eq!(MusicXmlFormat.read(&bytes).unwrap(), score);
    }

    #[test]
    fn several_navigation_marks_in_one_measure_round_trip() {
        let mut score = scale_score();
        score.parts[0].measures[0].navigation = vec![Navigation::Segno, Navigation::ToCoda];
        let bytes = MusicXmlFormat.write(&score).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        // The segno opens the measure, the To Coda closes it.
        let segno = text.find("<segno").unwrap();
        let to_coda = text.find("<words>To Coda</words>").unwrap();
        let first_note = text.find("<note").unwrap();
        assert!(segno < first_note && first_note < to_coda);
        assert_eq!(MusicXmlFormat.read(&bytes).unwrap(), score);
    }

    #[test]
    fn every_navigation_mark_round_trips_with_visible_text() {
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
        let measure = score.parts[0].measures[0].clone();
        score.parts[0].measures = marks
            .iter()
            .enumerate()
            .map(|(i, &mark)| {
                let mut m = measure.clone();
                m.number = i as u32 + 1;
                if i > 0 {
                    m.attributes = MeasureAttributes::default();
                }
                m.navigation = vec![mark];
                m
            })
            .collect();

        let bytes = MusicXmlFormat.write(&score).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("<segno"));
        assert!(text.contains("<coda"));
        assert!(text.contains("<words>D.S. al Coda</words>"));
        assert_eq!(MusicXmlFormat.read(&bytes).unwrap(), score);
    }

    #[test]
    fn reads_navigation_from_words_without_sound() {
        let words = |text: &str| {
            format!(
                r#"<direction placement="above"><direction-type><words>{text}</words></direction-type></direction>"#
            )
        };
        let rest = "<note><rest/><duration>4</duration><voice>1</voice><type>whole</type></note>";
        let measures = format!(
            r#"<measure number="1">{ATTRIBUTES}{rest}{}</measure>
<measure number="2">{rest}{}</measure>
<measure number="3">{rest}{}</measure>
<measure number="4">{}{rest}</measure>"#,
            words("D.C. al Fine"),
            words("Fine"),
            words("To Coda"),
            words("dal  segno"),
        );
        let score = read_measures(&measures);
        let marks: Vec<_> = score.parts[0]
            .measures
            .iter()
            .map(|m| m.navigation.clone())
            .collect();
        assert_eq!(
            marks,
            [
                vec![Navigation::DaCapoAlFine],
                vec![Navigation::Fine],
                vec![Navigation::ToCoda],
                vec![Navigation::DalSegno]
            ]
        );
    }
}
