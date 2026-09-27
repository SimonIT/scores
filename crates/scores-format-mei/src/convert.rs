use scores_core::{
    Articulation, BarlineType, Clef, Dynamic, Mode, Navigation, NoteValue, Ornament, Step, Tie,
    Wedge,
};

pub(crate) fn note_value_to_dur(value: NoteValue) -> u32 {
    match value {
        NoteValue::Whole => 1,
        NoteValue::Half => 2,
        NoteValue::Quarter => 4,
        NoteValue::Eighth => 8,
        NoteValue::Sixteenth => 16,
        NoteValue::ThirtySecond => 32,
        NoteValue::SixtyFourth => 64,
    }
}

pub(crate) fn dur_to_note_value(dur: u32) -> NoteValue {
    match dur {
        1 => NoteValue::Whole,
        2 => NoteValue::Half,
        4 => NoteValue::Quarter,
        8 => NoteValue::Eighth,
        16 => NoteValue::Sixteenth,
        32 => NoteValue::ThirtySecond,
        64 => NoteValue::SixtyFourth,
        _ => NoteValue::Quarter,
    }
}

pub(crate) fn step_to_pname(step: Step) -> &'static str {
    match step {
        Step::A => "a",
        Step::B => "b",
        Step::C => "c",
        Step::D => "d",
        Step::E => "e",
        Step::F => "f",
        Step::G => "g",
    }
}

pub(crate) fn pname_to_step(pname: &str) -> Step {
    match pname.to_ascii_lowercase().as_str() {
        "a" => Step::A,
        "b" => Step::B,
        "d" => Step::D,
        "e" => Step::E,
        "f" => Step::F,
        "g" => Step::G,
        _ => Step::C,
    }
}

pub(crate) fn alter_to_accid_ges(alter: i8) -> Option<&'static str> {
    match alter {
        2 => Some("ss"),
        1 => Some("s"),
        -1 => Some("f"),
        -2 => Some("ff"),
        _ => None,
    }
}

pub(crate) fn accid_ges_to_alter(accid: &str) -> i8 {
    match accid {
        "ss" => 2,
        "s" => 1,
        "f" => -1,
        "ff" => -2,
        _ => 0,
    }
}

pub(crate) fn tie_to_attr(tie: Tie) -> Option<&'static str> {
    match tie {
        Tie::None => None,
        Tie::Start => Some("i"),
        Tie::Stop => Some("t"),
        Tie::StartStop => Some("m"),
    }
}

pub(crate) fn attr_to_tie(value: &str) -> Tie {
    match value {
        "i" => Tie::Start,
        "t" => Tie::Stop,
        "m" => Tie::StartStop,
        _ => Tie::None,
    }
}

/// Formats a circle-of-fifths count as MEI's `key.sig` value: `"0"`,
/// `"<n>s"` for sharps, or `"<n>f"` for flats.
pub(crate) fn key_sig_to_attr(fifths: i8) -> String {
    match fifths.cmp(&0) {
        std::cmp::Ordering::Equal => "0".to_string(),
        std::cmp::Ordering::Greater => format!("{fifths}s"),
        std::cmp::Ordering::Less => format!("{}f", -fifths),
    }
}

pub(crate) fn attr_to_key_sig(value: &str) -> i8 {
    let value = value.trim();
    if let Some(n) = value.strip_suffix('s') {
        n.trim().parse().unwrap_or(0)
    } else if let Some(n) = value.strip_suffix('f') {
        -n.trim().parse().unwrap_or(0)
    } else {
        0
    }
}

pub(crate) fn mode_to_attr(mode: Mode) -> &'static str {
    match mode {
        Mode::Major => "major",
        Mode::Minor => "minor",
    }
}

pub(crate) fn attr_to_mode(value: &str) -> Mode {
    if value.eq_ignore_ascii_case("minor") {
        Mode::Minor
    } else {
        Mode::Major
    }
}

pub(crate) fn clef_to_shape_line(clef: Clef) -> (&'static str, i8) {
    match clef {
        Clef::Treble => ("G", 2),
        Clef::Bass => ("F", 4),
        Clef::Alto => ("C", 3),
        Clef::Tenor => ("C", 4),
        Clef::Percussion => ("perc", 2),
    }
}

pub(crate) fn shape_line_to_clef(shape: &str, line: i8) -> Clef {
    match shape {
        "G" => Clef::Treble,
        "F" => Clef::Bass,
        "C" => {
            if line == 4 {
                Clef::Tenor
            } else {
                Clef::Alto
            }
        }
        "perc" | "percussion" => Clef::Percussion,
        _ => Clef::Treble,
    }
}

/// `<dynam>` holds its dynamic mark as text content, matching real MEI.
pub(crate) fn dynamic_to_text(dynamic: &Dynamic) -> String {
    match dynamic {
        Dynamic::P => "p".into(),
        Dynamic::Pp => "pp".into(),
        Dynamic::Ppp => "ppp".into(),
        Dynamic::Pppp => "pppp".into(),
        Dynamic::Ppppp => "ppppp".into(),
        Dynamic::Pppppp => "pppppp".into(),
        Dynamic::F => "f".into(),
        Dynamic::Ff => "ff".into(),
        Dynamic::Fff => "fff".into(),
        Dynamic::Ffff => "ffff".into(),
        Dynamic::Fffff => "fffff".into(),
        Dynamic::Ffffff => "ffffff".into(),
        Dynamic::Mp => "mp".into(),
        Dynamic::Mf => "mf".into(),
        Dynamic::Sf => "sf".into(),
        Dynamic::Sfp => "sfp".into(),
        Dynamic::Sfpp => "sfpp".into(),
        Dynamic::Fp => "fp".into(),
        Dynamic::Rf => "rf".into(),
        Dynamic::Rfz => "rfz".into(),
        Dynamic::Sfz => "sfz".into(),
        Dynamic::Sffz => "sffz".into(),
        Dynamic::Fz => "fz".into(),
        Dynamic::N => "n".into(),
        Dynamic::Pf => "pf".into(),
        Dynamic::Sfzp => "sfzp".into(),
        Dynamic::Other(s) => s.clone(),
    }
}

pub(crate) fn text_to_dynamic(text: &str) -> Dynamic {
    match text {
        "p" => Dynamic::P,
        "pp" => Dynamic::Pp,
        "ppp" => Dynamic::Ppp,
        "pppp" => Dynamic::Pppp,
        "ppppp" => Dynamic::Ppppp,
        "pppppp" => Dynamic::Pppppp,
        "f" => Dynamic::F,
        "ff" => Dynamic::Ff,
        "fff" => Dynamic::Fff,
        "ffff" => Dynamic::Ffff,
        "fffff" => Dynamic::Fffff,
        "ffffff" => Dynamic::Ffffff,
        "mp" => Dynamic::Mp,
        "mf" => Dynamic::Mf,
        "sf" => Dynamic::Sf,
        "sfp" => Dynamic::Sfp,
        "sfpp" => Dynamic::Sfpp,
        "fp" => Dynamic::Fp,
        "rf" => Dynamic::Rf,
        "rfz" => Dynamic::Rfz,
        "sfz" => Dynamic::Sfz,
        "sffz" => Dynamic::Sffz,
        "fz" => Dynamic::Fz,
        "n" => Dynamic::N,
        "pf" => Dynamic::Pf,
        "sfzp" => Dynamic::Sfzp,
        other => Dynamic::Other(other.to_string()),
    }
}

/// Real MEI's `<hairpin>` has no "stop" form of its own (a hairpin spans a
/// range via `@tstamp2`/`@endid`); `"end"` is this plugin's own extension so
/// a [`Wedge::Stop`] has something to round-trip through.
pub(crate) fn wedge_to_form(wedge: Wedge) -> &'static str {
    match wedge {
        Wedge::CrescendoStart => "cres",
        Wedge::DiminuendoStart => "dim",
        Wedge::Stop => "end",
    }
}

pub(crate) fn form_to_wedge(form: &str) -> Option<Wedge> {
    match form {
        "cres" => Some(Wedge::CrescendoStart),
        "dim" => Some(Wedge::DiminuendoStart),
        "end" => Some(Wedge::Stop),
        _ => None,
    }
}

/// Written as an `<artic artic="...">` child of `<note>` (valid real MEI),
/// using `data.ARTICULATION` values where MEI has one. Breath marks and
/// caesuras are separate control events in real MEI (`<breath>`/
/// `<caesura>`), so `"breath"`/`"caes"` are this plugin's own codes.
pub(crate) fn articulation_to_code(articulation: &Articulation) -> String {
    match articulation {
        Articulation::Accent => "acc".into(),
        Articulation::StrongAccent => "marc".into(),
        Articulation::Staccato => "stacc".into(),
        Articulation::Tenuto => "ten".into(),
        Articulation::DetachedLegato => "ten-stacc".into(),
        Articulation::Staccatissimo => "stacciss".into(),
        Articulation::Spiccato => "spicc".into(),
        Articulation::Scoop => "scoop".into(),
        Articulation::Plop => "plop".into(),
        Articulation::Doit => "doit".into(),
        Articulation::Falloff => "fall".into(),
        Articulation::BreathMark => "breath".into(),
        Articulation::Caesura => "caes".into(),
        Articulation::Stress => "stress".into(),
        Articulation::Unstress => "unstress".into(),
        Articulation::SoftAccent => "acc-soft".into(),
        Articulation::Other(s) => s.clone(),
    }
}

pub(crate) fn code_to_articulation(code: &str) -> Articulation {
    match code {
        "acc" => Articulation::Accent,
        "marc" => Articulation::StrongAccent,
        "stacc" => Articulation::Staccato,
        "ten" => Articulation::Tenuto,
        // "detls"/"soft-acc" are what older versions of this plugin wrote.
        "ten-stacc" | "detls" => Articulation::DetachedLegato,
        "stacciss" => Articulation::Staccatissimo,
        "spicc" => Articulation::Spiccato,
        "scoop" => Articulation::Scoop,
        "plop" => Articulation::Plop,
        "doit" => Articulation::Doit,
        "fall" => Articulation::Falloff,
        "breath" => Articulation::BreathMark,
        "caes" => Articulation::Caesura,
        "stress" => Articulation::Stress,
        "unstress" => Articulation::Unstress,
        "acc-soft" | "soft-acc" => Articulation::SoftAccent,
        other => Articulation::Other(other.to_string()),
    }
}

/// Ornaments each map to a distinct MEI-ish child element of `<note>`.
/// `<trill>`, `<mordent form="lower|upper">` and `<turn form="lower|upper">`
/// are close to real MEI usage; everything else round-trips through this
/// plugin's own generic `<ornam type="...">` (with `count` for tremolo).
pub(crate) enum OrnamentElement {
    Trill,
    Mordent { upper: bool },
    Turn { upper: bool },
    Generic { kind: String, count: Option<u8> },
}

pub(crate) fn ornament_to_element(ornament: &Ornament) -> OrnamentElement {
    match ornament {
        Ornament::TrillMark => OrnamentElement::Trill,
        Ornament::Mordent => OrnamentElement::Mordent { upper: false },
        Ornament::InvertedMordent => OrnamentElement::Mordent { upper: true },
        Ornament::Turn => OrnamentElement::Turn { upper: true },
        Ornament::InvertedTurn => OrnamentElement::Turn { upper: false },
        Ornament::DelayedTurn => OrnamentElement::Generic {
            kind: "delayed-turn".into(),
            count: None,
        },
        Ornament::DelayedInvertedTurn => OrnamentElement::Generic {
            kind: "delayed-inverted-turn".into(),
            count: None,
        },
        Ornament::VerticalTurn => OrnamentElement::Generic {
            kind: "vertical-turn".into(),
            count: None,
        },
        Ornament::InvertedVerticalTurn => OrnamentElement::Generic {
            kind: "inverted-vertical-turn".into(),
            count: None,
        },
        Ornament::Shake => OrnamentElement::Generic {
            kind: "shake".into(),
            count: None,
        },
        Ornament::WavyLine => OrnamentElement::Generic {
            kind: "wavy-line".into(),
            count: None,
        },
        Ornament::Schleifer => OrnamentElement::Generic {
            kind: "schleifer".into(),
            count: None,
        },
        Ornament::Tremolo(n) => OrnamentElement::Generic {
            kind: "tremolo".into(),
            count: Some(*n),
        },
        Ornament::Haydn => OrnamentElement::Generic {
            kind: "haydn".into(),
            count: None,
        },
        Ornament::Other(s) => OrnamentElement::Generic {
            kind: s.clone(),
            count: None,
        },
    }
}

pub(crate) fn generic_kind_to_ornament(kind: &str, count: Option<u8>) -> Ornament {
    match kind {
        "delayed-turn" => Ornament::DelayedTurn,
        "delayed-inverted-turn" => Ornament::DelayedInvertedTurn,
        "vertical-turn" => Ornament::VerticalTurn,
        "inverted-vertical-turn" => Ornament::InvertedVerticalTurn,
        "shake" => Ornament::Shake,
        "wavy-line" => Ornament::WavyLine,
        "schleifer" => Ornament::Schleifer,
        "tremolo" => Ornament::Tremolo(count.unwrap_or(1)),
        "haydn" => Ornament::Haydn,
        other => Ornament::Other(other.to_string()),
    }
}

/// The barline style at `@right` (real MEI `data.BARRENDITION` values);
/// `repeat_end`/`repeat_start` are encoded separately (see `write.rs`), so
/// this only ever needs to express the plain visual styles. MEI has no
/// heavy-light, tick or short barline, so those three use this plugin's own
/// values.
pub(crate) fn barline_type_to_rend(kind: BarlineType) -> Option<&'static str> {
    match kind {
        BarlineType::Regular => None,
        BarlineType::Dotted => Some("dotted"),
        BarlineType::Dashed => Some("dashed"),
        BarlineType::Heavy => Some("heavy"),
        BarlineType::LightLight => Some("dbl"),
        BarlineType::LightHeavy => Some("end"),
        BarlineType::HeavyLight => Some("heavyLight"),
        BarlineType::HeavyHeavy => Some("dblheavy"),
        BarlineType::Tick => Some("tick"),
        BarlineType::Short => Some("short"),
        BarlineType::NoBarline => Some("invis"),
    }
}

/// Also accepts the MusicXML-style names (`lightLight`, ...) that older
/// versions of this plugin wrote.
pub(crate) fn rend_to_barline_type(rend: &str) -> BarlineType {
    match rend {
        "dotted" | "dbldotted" => BarlineType::Dotted,
        "dashed" | "dbldashed" => BarlineType::Dashed,
        "heavy" => BarlineType::Heavy,
        "dbl" | "lightLight" => BarlineType::LightLight,
        "end" | "lightHeavy" => BarlineType::LightHeavy,
        "heavyLight" => BarlineType::HeavyLight,
        "dblheavy" | "heavyHeavy" => BarlineType::HeavyHeavy,
        "tick" => BarlineType::Tick,
        "short" => BarlineType::Short,
        "invis" => BarlineType::NoBarline,
        _ => BarlineType::Regular,
    }
}

/// D.C./D.S./Coda/Segno/Fine are written as MEI 5's `<repeatMark
/// func="...">` (a control event after the staves): `func` gives the kind of
/// mark and the text content distinguishes e.g. "D.C." from "D.C. al Fine".
/// Returns `(func, text)`; segno/coda signs carry no text.
pub(crate) fn navigation_to_repeat_mark(navigation: Navigation) -> (&'static str, &'static str) {
    match navigation {
        Navigation::Segno => ("segno", ""),
        Navigation::Coda => ("coda", ""),
        Navigation::ToCoda => ("coda", "To Coda"),
        Navigation::DaCapo => ("daCapo", "D.C."),
        Navigation::DaCapoAlFine => ("daCapo", "D.C. al Fine"),
        Navigation::DaCapoAlCoda => ("daCapo", "D.C. al Coda"),
        Navigation::DalSegno => ("dalSegno", "D.S."),
        Navigation::DalSegnoAlFine => ("dalSegno", "D.S. al Fine"),
        Navigation::DalSegnoAlCoda => ("dalSegno", "D.S. al Coda"),
        Navigation::Fine => ("fine", "Fine"),
    }
}

/// Whether a navigation mark belongs at the end of its measure (jumps and
/// "Fine") rather than the start (segno/coda signs) - decides its `@tstamp`.
pub(crate) fn navigation_at_measure_end(navigation: Navigation) -> bool {
    !matches!(navigation, Navigation::Segno | Navigation::Coda)
}

/// Reads a `<repeatMark>` (from its `@func`, refined by its text), or - with
/// `func: None` - classifies free text such as a `<dir>`'s. Unrecognized
/// text yields `None`, so arbitrary directions aren't misread as jumps.
pub(crate) fn repeat_mark_to_navigation(func: Option<&str>, text: &str) -> Option<Navigation> {
    let lower = text.trim().to_lowercase();
    let al_fine = lower.contains("fine");
    let al_coda = lower.contains("coda");
    let da_capo = || {
        Some(if al_fine {
            Navigation::DaCapoAlFine
        } else if al_coda {
            Navigation::DaCapoAlCoda
        } else {
            Navigation::DaCapo
        })
    };
    let dal_segno = || {
        Some(if al_fine {
            Navigation::DalSegnoAlFine
        } else if al_coda {
            Navigation::DalSegnoAlCoda
        } else {
            Navigation::DalSegno
        })
    };
    match func {
        Some("segno") => Some(Navigation::Segno),
        Some("coda") if lower.starts_with("to") => Some(Navigation::ToCoda),
        Some("coda") => Some(Navigation::Coda),
        Some("daCapo") => da_capo(),
        Some("dalSegno") => dal_segno(),
        Some("fine") => Some(Navigation::Fine),
        _ => {
            if lower.starts_with("d.c.") || lower.starts_with("da capo") {
                da_capo()
            } else if lower.starts_with("d.s.") || lower.starts_with("dal segno") {
                dal_segno()
            } else {
                match lower.as_str() {
                    "fine" => Some(Navigation::Fine),
                    "to coda" => Some(Navigation::ToCoda),
                    "coda" | "\u{1d10c}" => Some(Navigation::Coda),
                    "segno" | "\u{1d10b}" => Some(Navigation::Segno),
                    _ => None,
                }
            }
        }
    }
}

/// Older versions of this plugin wrote an invented `<navigation type="...">`
/// element instead of `<repeatMark>`; still accepted on read.
pub(crate) fn type_to_navigation(value: &str) -> Option<Navigation> {
    Some(match value {
        "segno" => Navigation::Segno,
        "coda" => Navigation::Coda,
        "da-capo" => Navigation::DaCapo,
        "da-capo-al-fine" => Navigation::DaCapoAlFine,
        "da-capo-al-coda" => Navigation::DaCapoAlCoda,
        "dal-segno" => Navigation::DalSegno,
        "dal-segno-al-fine" => Navigation::DalSegnoAlFine,
        "dal-segno-al-coda" => Navigation::DalSegnoAlCoda,
        "fine" => Navigation::Fine,
        "to-coda" => Navigation::ToCoda,
        _ => return None,
    })
}
