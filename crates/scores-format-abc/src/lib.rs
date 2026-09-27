//! ABC notation plugin, built on the [`abc_parser`] crate for the actual
//! grammar (accidentals, octave marks, ties, chords, tuplets, broken rhythm,
//! decorations, comments, ...).
//!
//! Each [`scores_core::Part`] is read/written as one ABC tune (`X:1`, `X:2`,
//! ...). As required by the ABC standard (and enforced by `abc_parser`),
//! every tune needs at least an `X:` and a `T:` header field - a `T:` with
//! no title is fine, but an entirely missing one is a parse error.
//! `V:n` inline fields switch which voice subsequent notes belong to.
//!
//! Supported in both directions:
//! - dynamics (`!p!`, `!mf!`, `!sfz!`, ...) and hairpins (`!<(!`/
//!   `!crescendo(!`, `!>(!`/`!diminuendo(!`, `!<)!`/`!>)!`);
//! - articulations (`.`, `L`/`!accent!`, `!marcato!`, `!tenuto!`,
//!   `!wedge!`, `!breath!`, `!slide!`, up/down bow), ornaments (`T`, `M`,
//!   `P`, `~`, `!turn!`, `!invertedturn!`, `!trill(!` wavy line, `!/!`
//!   tremolos) and fermatas (`H`, also on rests);
//! - grace notes (`{...}`);
//! - chord symbols (`"Am7"C`, parsed with [`scores_core::ChordSymbol`]);
//! - barline styles (`|`, `||`, `|]`, `[|`, `.|`, `[|]`), repeats (`|:`,
//!   `:|`, `::`) and 1st/2nd endings (`|1`, `:|2`, `[1,2`);
//! - navigation marks (`S`, `O`, `!D.C.!`, `!D.S.!`, `!D.C.alfine!`,
//!   `!D.S.alcoda!`, `!fine!`, `!dacoda!`, ...), any number per measure.
//!
//! Not expressible, so lossy: an ending's extent and closed/open hook (ABC
//! only marks where an ending begins; the reader infers a bracket running
//! up to the next `:|` (closed) or else the next `||`/`|]`-style barline
//! (open), before any following ending or `|:`, defaulting to one open
//! measure), repeat counts above 2, dashed/heavy/tick/short
//! barline styles (written as the closest ABC style), articulations/
//! ornaments/dynamics with no ABC decoration, and marks on grace notes.
//! Mid-tune field changes other than `V:` (e.g. `[K:...]`) are ignored.
//! See the `read` and `write` module docs for the details.

mod read;
mod write;

use scores_core::{Dynamic, Format, FormatReader, FormatWriter, Result, Score};

pub struct AbcFormat;

impl FormatReader for AbcFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for AbcFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for AbcFormat {
    fn id(&self) -> &'static str {
        "abc"
    }

    fn name(&self) -> &'static str {
        "ABC notation"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["abc"]
    }
}

/// Maps a major key's circle-of-fifths position to its ABC tonic name.
pub(crate) const MAJOR_KEYS: [(&str, i8); 15] = [
    ("Cb", -7),
    ("Gb", -6),
    ("Db", -5),
    ("Ab", -4),
    ("Eb", -3),
    ("Bb", -2),
    ("F", -1),
    ("C", 0),
    ("G", 1),
    ("D", 2),
    ("A", 3),
    ("E", 4),
    ("B", 5),
    ("F#", 6),
    ("C#", 7),
];

pub(crate) fn fifths_to_tonic(fifths: i8) -> &'static str {
    MAJOR_KEYS
        .iter()
        .find(|(_, f)| *f == fifths)
        .map(|(name, _)| *name)
        .unwrap_or("C")
}

pub(crate) fn tonic_to_fifths(tonic: &str) -> i8 {
    MAJOR_KEYS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(tonic))
        .map(|(_, f)| *f)
        .unwrap_or(0)
}

/// Natural minor keys, in the same circle-of-fifths order as [`MAJOR_KEYS`]
/// (each entry is the relative minor of the major key at the same index).
pub(crate) const MINOR_KEYS: [(&str, i8); 15] = [
    ("Ab", -7),
    ("Eb", -6),
    ("Bb", -5),
    ("F", -4),
    ("C", -3),
    ("G", -2),
    ("D", -1),
    ("A", 0),
    ("E", 1),
    ("B", 2),
    ("F#", 3),
    ("C#", 4),
    ("G#", 5),
    ("D#", 6),
    ("A#", 7),
];

pub(crate) fn fifths_to_tonic_minor(fifths: i8) -> &'static str {
    MINOR_KEYS
        .iter()
        .find(|(_, f)| *f == fifths)
        .map(|(name, _)| *name)
        .unwrap_or("A")
}

pub(crate) fn tonic_to_fifths_minor(tonic: &str) -> i8 {
    MINOR_KEYS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(tonic))
        .map(|(_, f)| *f)
        .unwrap_or(0)
}

/// Every named [`Dynamic`] with the ABC decoration name it is written as
/// (`!mf!`). ABC 2.1 itself only lists `pppp`..`ffff`, `mp`, `mf` and
/// `sfz`; the rest use the same spelling, which abcm2ps/abcjs-style readers
/// accept as user-defined decorations. [`Dynamic::Other`] has no entry.
pub(crate) const DYNAMICS: [(Dynamic, &str); 26] = [
    (Dynamic::P, "p"),
    (Dynamic::Pp, "pp"),
    (Dynamic::Ppp, "ppp"),
    (Dynamic::Pppp, "pppp"),
    (Dynamic::Ppppp, "ppppp"),
    (Dynamic::Pppppp, "pppppp"),
    (Dynamic::F, "f"),
    (Dynamic::Ff, "ff"),
    (Dynamic::Fff, "fff"),
    (Dynamic::Ffff, "ffff"),
    (Dynamic::Fffff, "fffff"),
    (Dynamic::Ffffff, "ffffff"),
    (Dynamic::Mp, "mp"),
    (Dynamic::Mf, "mf"),
    (Dynamic::Sf, "sf"),
    (Dynamic::Sfp, "sfp"),
    (Dynamic::Sfpp, "sfpp"),
    (Dynamic::Fp, "fp"),
    (Dynamic::Rf, "rf"),
    (Dynamic::Rfz, "rfz"),
    (Dynamic::Sfz, "sfz"),
    (Dynamic::Sffz, "sffz"),
    (Dynamic::Fz, "fz"),
    (Dynamic::N, "n"),
    (Dynamic::Pf, "pf"),
    (Dynamic::Sfzp, "sfzp"),
];

/// The ABC decoration name for `dynamic`, if it has one.
pub(crate) fn dynamic_name(dynamic: &Dynamic) -> Option<&'static str> {
    DYNAMICS
        .iter()
        .find(|(d, _)| d == dynamic)
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::{
        Articulation, BarlineType, ChordSymbol, Clef, Duration, Dynamic, Ending, Event,
        KeySignature, Measure, MeasureAttributes, Navigation, Note, NoteValue, Ornament, Part,
        Pitch, Rest, Step, Tie, TimeSignature, Wedge,
    };

    const TWINKLE: &str =
        "X:1\nT:Twinkle Twinkle\nC:Trad.\nM:4/4\nL:1/8\nK:C\nCC GG|AA G2|FF EE|DD C2|]\n";

    #[test]
    fn reads_a_simple_tune() {
        let score = AbcFormat.read(TWINKLE.as_bytes()).unwrap();
        assert_eq!(score.title.as_deref(), Some("Twinkle Twinkle"));
        assert_eq!(score.composer.as_deref(), Some("Trad."));
        assert_eq!(score.parts.len(), 1);

        let part = &score.parts[0];
        assert_eq!(part.measures.len(), 4);

        let first = &part.measures[0];
        assert_eq!(first.attributes.time_signature.unwrap().beats, 4);
        assert_eq!(first.attributes.key_signature.unwrap().fifths, 0);
        assert_eq!(first.voices[0].events.len(), 4);

        let Event::Note(note) = &first.voices[0].events[0] else {
            panic!("expected a note")
        };
        assert_eq!(note.pitch.step, Step::C);
        assert_eq!(note.pitch.octave, 4);
        assert_eq!(note.duration.base, NoteValue::Eighth);
    }

    #[test]
    fn round_trips_through_write_and_read() {
        let original = AbcFormat.read(TWINKLE.as_bytes()).unwrap();
        let written = AbcFormat.write(&original).unwrap();
        let reread = AbcFormat.read(&written).unwrap();

        assert_eq!(reread.title, original.title);
        assert_eq!(reread.parts.len(), original.parts.len());
        assert_eq!(
            reread.parts[0].measures.len(),
            original.parts[0].measures.len()
        );
        for (a, b) in original.parts[0]
            .measures
            .iter()
            .zip(&reread.parts[0].measures)
        {
            assert_eq!(a.voices[0].events, b.voices[0].events);
        }
    }

    #[test]
    fn ties_and_accidentals_round_trip() {
        let text = "X:1\nT:Accidentals\nK:D\n^F2-^F2 =c2 _B2|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let events = &score.parts[0].measures[0].voices[0].events;

        let Event::Note(first) = &events[0] else {
            panic!("expected a note")
        };
        assert_eq!(first.tie, Tie::Start);
        let Event::Note(second) = &events[1] else {
            panic!("expected a note")
        };
        assert_eq!(second.tie, Tie::Stop);
        assert_eq!(second.pitch.alter, 1);

        let written = AbcFormat.write(&score).unwrap();
        let reread = AbcFormat.read(&written).unwrap();
        assert_eq!(
            reread.parts[0].measures[0].voices[0].events,
            score.parts[0].measures[0].voices[0].events
        );
    }

    #[test]
    fn decorations_map_to_articulations_ornaments_fermata_and_navigation() {
        let text = "X:1\nT:Deco\nM:4/4\nL:1/8\nK:C\nTC2 LD2 HE2 F2 O|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let events = &score.parts[0].measures[0].voices[0].events;

        let Event::Note(c) = &events[0] else { panic!() };
        assert!(c.ornaments.contains(&scores_core::Ornament::TrillMark));
        let Event::Note(d) = &events[1] else { panic!() };
        assert!(d.articulations.contains(&scores_core::Articulation::Accent));
        let Event::Note(e) = &events[2] else { panic!() };
        assert!(e.fermata);

        assert_eq!(
            score.parts[0].measures[0].navigation,
            vec![scores_core::Navigation::Coda]
        );

        let written = AbcFormat.write(&score).unwrap();
        let reread = AbcFormat.read(&written).unwrap();
        let reread_events = &reread.parts[0].measures[0].voices[0].events;
        let Event::Note(c2) = &reread_events[0] else {
            panic!()
        };
        assert!(c2.ornaments.contains(&scores_core::Ornament::TrillMark));
        assert_eq!(
            reread.parts[0].measures[0].navigation,
            vec![scores_core::Navigation::Coda]
        );
    }

    #[test]
    fn repeats_and_endings_round_trip() {
        let text = "X:1\nT:Rep\nM:4/4\nL:1/8\nK:C\nC2D2E2F2 |1 G2A2 :|2 B2c2 |]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        assert_eq!(score.parts[0].measures.len(), 3);

        // An ending belongs to the measure it spans, not the bar before it.
        let measures = &score.parts[0].measures;
        assert_eq!(measures[0].barline.ending, None);
        assert_eq!(
            measures[1].barline.ending.as_ref().unwrap().numbers,
            vec![1]
        );
        assert_eq!(measures[1].barline.repeat_end, Some(2));
        assert_eq!(
            measures[2].barline.ending.as_ref().unwrap().numbers,
            vec![2]
        );

        let written = AbcFormat.write(&score).unwrap();
        let reread = AbcFormat.read(&written).unwrap();
        assert_eq!(reread.parts[0].measures, score.parts[0].measures);
    }

    #[test]
    fn grace_notes_round_trip() {
        let text = "X:1\nT:Grace\nM:4/4\nL:1/8\nK:C\n{AB}C2 D2 E2 F2|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let events = &score.parts[0].measures[0].voices[0].events;

        // Two grace notes (A, B) followed by the four main notes.
        assert_eq!(events.len(), 6);
        let Event::Note(grace_a) = &events[0] else {
            panic!()
        };
        assert!(grace_a.grace);
        assert_eq!(grace_a.pitch.step, Step::A);
        let Event::Note(grace_b) = &events[1] else {
            panic!()
        };
        assert!(grace_b.grace);
        assert_eq!(grace_b.pitch.step, Step::B);
        let Event::Note(main) = &events[2] else {
            panic!()
        };
        assert!(!main.grace);
        assert_eq!(main.pitch.step, Step::C);

        let written = AbcFormat.write(&score).unwrap();
        let text = String::from_utf8(written.clone()).unwrap();
        assert!(text.contains('{') && text.contains('}'));

        let reread = AbcFormat.read(&written).unwrap();
        let reread_events = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(reread_events.len(), 6);
        let Event::Note(reread_grace_a) = &reread_events[0] else {
            panic!()
        };
        assert!(reread_grace_a.grace);
        assert_eq!(reread_grace_a.pitch.step, Step::A);
    }

    #[test]
    fn multiple_voices_round_trip_independently() {
        let text = "X:1\nT:Multi\nM:4/4\nL:1/8\nK:C\nV:1\nC2D2E2F2|]\nV:2\nC,2C,2C,2C,2|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let part = &score.parts[0];
        assert_eq!(part.measures[0].voices.len(), 2);

        let voice1 = part.measures[0]
            .voices
            .iter()
            .find(|v| v.number == 1)
            .unwrap();
        let Event::Note(first) = &voice1.events[0] else {
            panic!()
        };
        assert_eq!(first.pitch.step, Step::C);
        assert_eq!(first.pitch.octave, 4);

        let voice2 = part.measures[0]
            .voices
            .iter()
            .find(|v| v.number == 2)
            .unwrap();
        let Event::Note(bass) = &voice2.events[0] else {
            panic!()
        };
        assert_eq!(bass.pitch.step, Step::C);
        assert_eq!(bass.pitch.octave, 3);

        let written = AbcFormat.write(&score).unwrap();
        let reread = AbcFormat.read(&written).unwrap();
        assert_eq!(reread.parts[0].measures[0].voices.len(), 2);
    }

    fn note(step: Step) -> Note {
        Note::new(
            Pitch::new(step, 0, 4),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        )
    }

    fn one_part(measures: Vec<Measure>) -> Score {
        let mut measures = measures;
        measures[0].attributes = MeasureAttributes {
            time_signature: Some(TimeSignature::default()),
            key_signature: Some(KeySignature::default()),
            clef: Some(Clef::Treble),
            tempo: None,
        };
        Score {
            title: Some("T".into()),
            composer: None,
            parts: vec![Part {
                id: "P1".into(),
                name: Some("T".into()),
                measures,
            }],
        }
    }

    fn events(score: &Score, measure: usize) -> &[Event] {
        &score.parts[0].measures[measure].voices[0].events
    }

    fn assert_round_trips(score: &Score) {
        let written = AbcFormat.write(score).unwrap();
        let reread = AbcFormat.read(&written).unwrap();
        assert_eq!(
            reread.parts[0].measures,
            score.parts[0].measures,
            "written as:\n{}",
            String::from_utf8_lossy(&written)
        );
    }

    #[test]
    fn reads_dynamics_and_hairpins() {
        let text =
            "X:1\nT:Dyn\nM:4/4\nL:1/4\nK:C\n!p!C !<(!D !crescendo)!!f!E !>(!F|!>)!!sfz!G4|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let notes: Vec<&Note> = (0..2)
            .flat_map(|m| events(&score, m))
            .map(|e| match e {
                Event::Note(n) => n,
                _ => panic!("expected notes"),
            })
            .collect();
        assert_eq!(notes[0].dynamic, Some(Dynamic::P));
        assert_eq!(notes[1].wedge, Some(Wedge::CrescendoStart));
        assert_eq!(notes[2].wedge, Some(Wedge::Stop));
        assert_eq!(notes[2].dynamic, Some(Dynamic::F));
        assert_eq!(notes[3].wedge, Some(Wedge::DiminuendoStart));
        assert_eq!(notes[4].wedge, Some(Wedge::Stop));
        assert_eq!(notes[4].dynamic, Some(Dynamic::Sfz));
        assert!(notes.iter().all(|n| n.articulations.is_empty()));
    }

    #[test]
    fn dynamics_and_hairpins_round_trip() {
        let mut notes: Vec<Note> = [Step::C, Step::D, Step::E, Step::F]
            .into_iter()
            .map(note)
            .collect();
        notes[0].dynamic = Some(Dynamic::Mp);
        notes[0].wedge = Some(Wedge::DiminuendoStart);
        notes[1].wedge = Some(Wedge::Stop);
        notes[1].dynamic = Some(Dynamic::Fp);
        notes[2].wedge = Some(Wedge::CrescendoStart);
        notes[3].wedge = Some(Wedge::Stop);
        notes[3].dynamic = Some(Dynamic::Ff);
        let mut measure = Measure::single_voice(
            1,
            MeasureAttributes::default(),
            notes.into_iter().map(Event::Note).collect(),
        );
        measure.barline.kind = BarlineType::LightHeavy;
        let score = one_part(vec![measure]);

        let text = String::from_utf8(AbcFormat.write(&score).unwrap()).unwrap();
        assert!(text.contains("!mp!") && text.contains("!>(!") && text.contains("!>)!"));
        assert!(text.contains("!<(!") && text.contains("!<)!"));
        assert_round_trips(&score);
    }

    #[test]
    fn articulations_ornaments_and_fermatas_round_trip() {
        let mut a = note(Step::C);
        a.articulations = vec![
            Articulation::Staccato,
            Articulation::Accent,
            Articulation::StrongAccent,
            Articulation::Tenuto,
            Articulation::Staccatissimo,
            Articulation::BreathMark,
            Articulation::Scoop,
            Articulation::Other("up-bow".into()),
            Articulation::Other("snap".into()),
        ];
        let mut b = note(Step::D);
        b.ornaments = vec![
            Ornament::TrillMark,
            Ornament::Mordent,
            Ornament::InvertedMordent,
            Ornament::Turn,
            Ornament::InvertedTurn,
            Ornament::WavyLine,
            Ornament::Tremolo(3),
            Ornament::Other("roll".into()),
        ];
        b.fermata = true;
        let mut rest = Rest::new(Duration::new(NoteValue::Quarter));
        rest.fermata = true;
        let mut chord = vec![note(Step::E), note(Step::G)];
        chord[0].articulations = vec![Articulation::Tenuto];
        chord[1].articulations = vec![Articulation::Tenuto];
        let mut measure = Measure::single_voice(
            1,
            MeasureAttributes::default(),
            vec![
                Event::Note(a),
                Event::Note(b),
                Event::Rest(rest),
                Event::Chord(chord),
            ],
        );
        measure.barline.kind = BarlineType::LightHeavy;
        assert_round_trips(&one_part(vec![measure]));
    }

    #[test]
    fn reads_named_decorations_and_decorations_inside_chords() {
        let text =
            "X:1\nT:Deco\nL:1/4\nK:C\n!tenuto!C !uppermordent!D !invertedfermata!z [!>!C.E]|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let events = events(&score, 0);
        let Event::Note(c) = &events[0] else { panic!() };
        assert_eq!(c.articulations, vec![Articulation::Tenuto]);
        let Event::Note(d) = &events[1] else { panic!() };
        assert_eq!(d.ornaments, vec![Ornament::InvertedMordent]);
        let Event::Rest(z) = &events[2] else { panic!() };
        assert!(z.fermata);
        let Event::Chord(chord) = &events[3] else {
            panic!()
        };
        assert_eq!(chord[0].articulations, vec![Articulation::Accent]);
        assert_eq!(chord[1].articulations, vec![Articulation::Staccato]);
    }

    #[test]
    fn reads_chord_symbols_but_not_annotations() {
        let text = "X:1\nT:Chords\nL:1/4\nK:C\n\"Am7\"A \"^rit.\"B \"G/B\"z \"N.C.\"[CE] {c}\"F#m7b5\"D|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let events = events(&score, 0);
        assert_eq!(events[0].chord_symbol(), ChordSymbol::parse("Am7").as_ref());
        assert_eq!(events[1].chord_symbol(), None);
        assert_eq!(events[2].chord_symbol(), ChordSymbol::parse("G/B").as_ref());
        assert_eq!(events[3].chord_symbol(), Some(&ChordSymbol::no_chord()));
        let Event::Chord(chord) = &events[3] else {
            panic!()
        };
        assert_eq!(chord[1].chord_symbol, None);
        // The grace note leaves the chord symbol for the main note.
        assert_eq!(events[4].chord_symbol(), None);
        assert_eq!(
            events[5].chord_symbol(),
            ChordSymbol::parse("F#m7b5").as_ref()
        );
    }

    #[test]
    fn chord_symbols_round_trip() {
        let mut first = Event::Note(note(Step::A));
        first.set_chord_symbol(ChordSymbol::parse("Bbmaj7/D"));
        let mut rest = Event::Rest(Rest::new(Duration::new(NoteValue::Quarter)));
        rest.set_chord_symbol(ChordSymbol::parse("Dsus4"));
        let mut chord = Event::Chord(vec![note(Step::C), note(Step::E)]);
        chord.set_chord_symbol(ChordSymbol::parse("C"));
        let mut grace = note(Step::B);
        grace.grace = true;
        let mut after_grace = Event::Note(note(Step::C));
        after_grace.set_chord_symbol(ChordSymbol::parse("G7b9"));
        let mut measure = Measure::single_voice(
            1,
            MeasureAttributes::default(),
            vec![first, rest, chord, Event::Note(grace), after_grace],
        );
        measure.barline.kind = BarlineType::LightHeavy;
        let score = one_part(vec![measure]);

        let text = String::from_utf8(AbcFormat.write(&score).unwrap()).unwrap();
        assert!(text.contains("\"Bbmaj7/D\"A") && text.contains("{B2}"));
        assert_round_trips(&score);
    }

    #[test]
    fn reads_barline_styles() {
        let text = "X:1\nT:Bars\nL:1/4\nK:C\nC4|D4||E4.|F4[|]G4[|A4|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let kinds: Vec<BarlineType> = score.parts[0]
            .measures
            .iter()
            .map(|m| m.barline.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                BarlineType::Regular,
                BarlineType::LightLight,
                BarlineType::Dotted,
                BarlineType::NoBarline,
                BarlineType::HeavyLight,
                BarlineType::LightHeavy,
            ]
        );
    }

    #[test]
    fn barline_styles_repeats_and_endings_round_trip() {
        let kinds = [
            BarlineType::Dotted,
            BarlineType::LightLight,
            BarlineType::HeavyLight,
            BarlineType::Regular,
            BarlineType::NoBarline,
            BarlineType::Regular,
            BarlineType::LightHeavy,
        ];
        let mut measures: Vec<Measure> = kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| {
                let mut m = Measure::single_voice(
                    i as u32 + 1,
                    MeasureAttributes::default(),
                    vec![Event::Note(note(Step::C))],
                );
                m.barline.kind = *kind;
                m
            })
            .collect();
        measures[0].barline.repeat_start = true;
        measures[2].barline.repeat_start = true;
        measures[3].barline.ending = Some(Ending {
            numbers: vec![1, 2],
            measures: 1,
            closed: true,
        });
        measures[3].barline.repeat_end = Some(2);
        measures[4].barline.ending = Some(Ending {
            numbers: vec![3],
            measures: 1,
            closed: false,
        });
        let score = one_part(measures);

        let text = String::from_utf8(AbcFormat.write(&score).unwrap()).unwrap();
        assert!(text.contains("|:") && text.contains("[1,2") && text.contains(":|3"));
        assert!(text.contains(".|") && text.contains("[|]"));
        assert_round_trips(&score);
    }

    #[test]
    fn leading_repeat_and_ending_ranges() {
        let text = "X:1\nT:Lead\nL:1/4\nK:C\n|: C4 |1-2 D4 :|\n[3 E4 |]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let measures = &score.parts[0].measures;
        assert_eq!(measures.len(), 3);
        assert!(measures[0].barline.repeat_start);
        assert_eq!(
            measures[1].barline.ending.as_ref().unwrap().numbers,
            vec![1, 2]
        );
        assert_eq!(measures[1].barline.repeat_end, Some(2));
        assert_eq!(
            measures[2].barline.ending.as_ref().unwrap().numbers,
            vec![3]
        );
        assert_round_trips(&score);
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
        let mut measures: Vec<Measure> = marks
            .iter()
            .enumerate()
            .map(|(i, mark)| {
                let mut m = Measure::single_voice(
                    i as u32 + 1,
                    MeasureAttributes::default(),
                    vec![Event::Note(note(Step::D))],
                );
                m.navigation = vec![*mark];
                m
            })
            .collect();
        measures.last_mut().unwrap().barline.kind = BarlineType::LightHeavy;
        let score = one_part(measures);
        assert_round_trips(&score);

        let text = "X:1\nT:Nav\nL:1/4\nK:C\n!segno!C4|D4!D.S.alcoda!|!coda!E4 !fine!|]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let read: Vec<Vec<Navigation>> = score.parts[0]
            .measures
            .iter()
            .map(|m| m.navigation.clone())
            .collect();
        assert_eq!(
            read,
            vec![
                vec![Navigation::Segno],
                vec![Navigation::DalSegnoAlCoda],
                vec![Navigation::Coda, Navigation::Fine],
            ]
        );
    }

    #[test]
    fn several_navigation_marks_in_one_measure_round_trip() {
        let mut measures: Vec<Measure> = (1..=2)
            .map(|i| {
                Measure::single_voice(
                    i,
                    MeasureAttributes::default(),
                    vec![Event::Note(note(Step::E))],
                )
            })
            .collect();
        measures[0].navigation = vec![Navigation::Segno, Navigation::ToCoda];
        measures[1].navigation = vec![Navigation::Coda, Navigation::DalSegnoAlCoda];
        measures[1].barline.kind = BarlineType::LightHeavy;
        let score = one_part(measures);

        let text = String::from_utf8(AbcFormat.write(&score).unwrap()).unwrap();
        assert!(text.contains("S") && text.contains("!dacoda!"));
        assert_round_trips(&score);
    }

    #[test]
    fn ending_extents_are_inferred() {
        // 1st ending: two measures up to the `:|` (closed); 2nd ending: two
        // measures up to the `||` (open). In the second section, a
        // one-measure 1st ending closes at its own `:|`, and the 2nd ending
        // stops at the final `|]`.
        let text = "X:1\nT:Ext\nL:1/4\nK:C\n|: C4 |1 D4 | E4 :|2 F4 | G4 ||\n\
                    |: A4 |1 B4 :|2 c4 |]\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let endings: Vec<Option<(Vec<u8>, u32, bool)>> = score.parts[0]
            .measures
            .iter()
            .map(|m| {
                m.barline
                    .ending
                    .as_ref()
                    .map(|e| (e.numbers.clone(), e.measures, e.closed))
            })
            .collect();
        assert_eq!(
            endings,
            vec![
                None,
                Some((vec![1], 2, true)),
                None,
                Some((vec![2], 2, false)),
                None,
                None,
                Some((vec![1], 1, true)),
                Some((vec![2], 1, false)),
            ]
        );
        assert_round_trips(&score);
    }

    #[test]
    fn an_unterminated_last_ending_covers_one_measure() {
        let text = "X:1\nT:Ext\nL:1/4\nK:C\n|: C4 |1 D4 :|2 E4 | F4 | G4\n";
        let score = AbcFormat.read(text.as_bytes()).unwrap();
        let ending = score.parts[0].measures[2].barline.ending.as_ref().unwrap();
        assert_eq!((ending.measures, ending.closed), (1, false));
    }
}
