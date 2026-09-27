use guitarpro::model::optimized::beat::Duration as OptDuration;
use guitarpro::model::optimized::metadata::{KeySignature as OptKeySignature, Mode as OptMode};
use guitarpro::model::optimized::note::{
    NoteValue as OptNoteValue, Pitch as OptPitch, PitchStep as OptPitchStep,
};
use guitarpro::model::optimized::track::Clef as OptClef;
use scores_core::{
    Articulation, BarlineType, ChordRoot, Clef, Duration, Dynamic, KeySignature, Mode, Navigation,
    NoteValue, Ornament, Pitch, Step, Wedge,
};

pub(crate) fn note_value_to_model(value: OptNoteValue) -> NoteValue {
    match value {
        OptNoteValue::Whole => NoteValue::Whole,
        OptNoteValue::Half => NoteValue::Half,
        OptNoteValue::Quarter => NoteValue::Quarter,
        OptNoteValue::Eighth => NoteValue::Eighth,
        OptNoteValue::Sixteenth => NoteValue::Sixteenth,
        OptNoteValue::ThirtySecond => NoteValue::ThirtySecond,
        OptNoteValue::SixtyFourth | OptNoteValue::HundredTwentyEighth | OptNoteValue::Other(_) => {
            NoteValue::SixtyFourth
        }
    }
}

pub(crate) fn note_value_to_optimized(value: NoteValue) -> OptNoteValue {
    match value {
        NoteValue::Whole => OptNoteValue::Whole,
        NoteValue::Half => OptNoteValue::Half,
        NoteValue::Quarter => OptNoteValue::Quarter,
        NoteValue::Eighth => OptNoteValue::Eighth,
        NoteValue::Sixteenth => OptNoteValue::Sixteenth,
        NoteValue::ThirtySecond => OptNoteValue::ThirtySecond,
        NoteValue::SixtyFourth => OptNoteValue::SixtyFourth,
    }
}

/// Tuplets are dropped: the `guitarpro` crate's mscz converter (0.4.3)
/// hard-codes `tuplet: None` when reading and never looks at the field when
/// writing, regardless of what's stored here - see the module docs.
pub(crate) fn duration_to_model(d: &OptDuration) -> Duration {
    Duration {
        base: note_value_to_model(d.base),
        dots: d.dots,
        tuplet: None,
    }
}

pub(crate) fn duration_to_optimized(d: &Duration) -> OptDuration {
    OptDuration {
        base: note_value_to_optimized(d.base),
        dots: d.dots,
        tuplet: None,
    }
}

pub(crate) fn step_to_optimized(step: Step) -> OptPitchStep {
    match step {
        Step::A => OptPitchStep::A,
        Step::B => OptPitchStep::B,
        Step::C => OptPitchStep::C,
        Step::D => OptPitchStep::D,
        Step::E => OptPitchStep::E,
        Step::F => OptPitchStep::F,
        Step::G => OptPitchStep::G,
    }
}

pub(crate) fn step_from_optimized(step: OptPitchStep) -> Step {
    match step {
        OptPitchStep::A => Step::A,
        OptPitchStep::B => Step::B,
        OptPitchStep::C => Step::C,
        OptPitchStep::D => Step::D,
        OptPitchStep::E => Step::E,
        OptPitchStep::F => Step::F,
        OptPitchStep::G => Step::G,
    }
}

pub(crate) fn pitch_to_model(p: OptPitch) -> Pitch {
    Pitch::new(step_from_optimized(p.step), p.alter, p.octave as i8)
}

pub(crate) fn pitch_to_optimized(p: Pitch) -> OptPitch {
    OptPitch {
        step: step_to_optimized(p.step),
        alter: p.alter,
        octave: p.octave.clamp(0, 9) as u8,
    }
}

pub(crate) fn clef_to_model(clef: OptClef) -> Clef {
    match clef {
        OptClef::Treble | OptClef::Tab => Clef::Treble,
        OptClef::Bass => Clef::Bass,
        OptClef::Alto => Clef::Alto,
        OptClef::Tenor => Clef::Tenor,
        OptClef::Percussion => Clef::Percussion,
    }
}

pub(crate) fn clef_to_optimized(clef: Clef) -> OptClef {
    match clef {
        Clef::Treble => OptClef::Treble,
        Clef::Bass => OptClef::Bass,
        Clef::Alto => OptClef::Alto,
        Clef::Tenor => OptClef::Tenor,
        Clef::Percussion => OptClef::Percussion,
    }
}

/// Canonical (step, alter) spelling for a major-key tonic at each
/// circle-of-fifths position, `-7..=7` (Cb major .. C# major).
const MAJOR_TONICS: [(Step, i8, i8); 15] = [
    (Step::C, -1, -7),
    (Step::G, -1, -6),
    (Step::D, -1, -5),
    (Step::A, -1, -4),
    (Step::E, -1, -3),
    (Step::B, -1, -2),
    (Step::F, 0, -1),
    (Step::C, 0, 0),
    (Step::G, 0, 1),
    (Step::D, 0, 2),
    (Step::A, 0, 3),
    (Step::E, 0, 4),
    (Step::B, 0, 5),
    (Step::F, 1, 6),
    (Step::C, 1, 7),
];

fn fifths_for_major_tonic(step: Step, alter: i8) -> i8 {
    MAJOR_TONICS
        .iter()
        .find(|&&(s, a, _)| s == step && a == alter)
        .map(|&(_, _, f)| f)
        .unwrap_or(0)
}

fn major_tonic_for_fifths(fifths: i8) -> (Step, i8) {
    MAJOR_TONICS
        .iter()
        .find(|&&(_, _, f)| f == fifths)
        .map(|&(s, a, _)| (s, a))
        .unwrap_or((Step::C, 0))
}

/// Circle-of-fifths distance of each church mode from Ionian (major) on the
/// same tonic - e.g. Mixolydian is a fifth flatter than major on the same
/// root, Lydian a fifth sharper. Our own model only distinguishes
/// Major/Minor, so every other mode is folded into whichever fifths count
/// this produces (still musically correct - it's the same key signature -
/// just without the modal name).
fn modal_offset(mode: OptMode) -> i8 {
    match mode {
        OptMode::Lydian => 1,
        OptMode::Major => 0,
        OptMode::Mixolydian => -1,
        OptMode::Dorian => -2,
        OptMode::Minor => -3,
        OptMode::Phrygian => -4,
        OptMode::Locrian => -5,
    }
}

pub(crate) fn key_signature_to_model(k: OptKeySignature) -> KeySignature {
    let base = fifths_for_major_tonic(step_from_optimized(k.root.step), k.root.alter);
    let fifths = (base + modal_offset(k.mode)).clamp(-7, 7);
    let mode = if k.mode == OptMode::Minor {
        Mode::Minor
    } else {
        Mode::Major
    };
    KeySignature { fifths, mode }
}

pub(crate) fn key_signature_to_optimized(k: KeySignature) -> OptKeySignature {
    let minor = k.mode == Mode::Minor;
    // The relative-minor tonic at fifths `f` is the major tonic at `f + 3`
    // (a minor third up) - same key signature, different starting pitch.
    let lookup_fifths = if minor {
        (k.fifths + 3).clamp(-7, 7)
    } else {
        k.fifths
    };
    let (step, alter) = major_tonic_for_fifths(lookup_fifths);
    OptKeySignature {
        root: OptPitch {
            step: step_to_optimized(step),
            alter,
            octave: 4,
        },
        mode: if minor {
            OptMode::Minor
        } else {
            OptMode::Major
        },
    }
}

// ---------------------------------------------------------------------------
// MSCX vocabulary for the elements `guitarpro`'s converter drops, which
// `crate::mscx` reads from / splices into the raw XML itself.
// ---------------------------------------------------------------------------

/// Dynamic subtypes MuseScore 3/4 know by name (`<Dynamic><subtype>`).
/// Anything else is written as `other-dynamics` with the name as its text.
const MSCX_DYNAMICS: [&str; 30] = [
    "pppppp", "ppppp", "pppp", "ppp", "pp", "p", "mp", "mf", "f", "ff", "fff", "ffff", "fffff",
    "ffffff", "fp", "pf", "sf", "sfz", "sff", "sffz", "sfp", "sfpp", "rfz", "rf", "fz", "m", "r",
    "s", "z", "n",
];

/// The conventional marking text of a dynamic (`Other` verbatim).
pub(crate) fn dynamic_name(dynamic: &Dynamic) -> &str {
    match dynamic {
        Dynamic::P => "p",
        Dynamic::Pp => "pp",
        Dynamic::Ppp => "ppp",
        Dynamic::Pppp => "pppp",
        Dynamic::Ppppp => "ppppp",
        Dynamic::Pppppp => "pppppp",
        Dynamic::F => "f",
        Dynamic::Ff => "ff",
        Dynamic::Fff => "fff",
        Dynamic::Ffff => "ffff",
        Dynamic::Fffff => "fffff",
        Dynamic::Ffffff => "ffffff",
        Dynamic::Mp => "mp",
        Dynamic::Mf => "mf",
        Dynamic::Sf => "sf",
        Dynamic::Sfp => "sfp",
        Dynamic::Sfpp => "sfpp",
        Dynamic::Fp => "fp",
        Dynamic::Rf => "rf",
        Dynamic::Rfz => "rfz",
        Dynamic::Sfz => "sfz",
        Dynamic::Sffz => "sffz",
        Dynamic::Fz => "fz",
        Dynamic::N => "n",
        Dynamic::Pf => "pf",
        Dynamic::Sfzp => "sfzp",
        Dynamic::Other(text) => text,
    }
}

/// Inverse of [`dynamic_name`]; unknown names become [`Dynamic::Other`].
pub(crate) fn dynamic_from_name(name: &str) -> Dynamic {
    match name {
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

/// Whether `name` is a `<subtype>` MuseScore understands (vs. needing the
/// `other-dynamics` + text form).
pub(crate) fn is_mscx_dynamic_subtype(name: &str) -> bool {
    MSCX_DYNAMICS.contains(&name)
}

/// MuseScore's default playback velocity for a dynamic subtype, written
/// alongside it so the file plays back like one MuseScore saved itself.
pub(crate) fn dynamic_velocity(name: &str) -> Option<u8> {
    Some(match name {
        "pppppp" => 1,
        "ppppp" => 5,
        "pppp" => 10,
        "ppp" => 16,
        "pp" => 33,
        "p" | "pf" => 49,
        "mp" => 64,
        "mf" => 80,
        "f" | "fp" => 96,
        "ff" | "sf" | "sfz" | "sfp" | "sfpp" | "rfz" | "rf" | "fz" => 112,
        "fff" | "sff" | "sffz" => 126,
        "ffff" | "fffff" | "ffffff" => 127,
        _ => return None,
    })
}

/// `<HairPin><subtype>`: 0/1 are the hairpin shapes, 2/3 the equivalent
/// "cresc."/"dim." text lines - the model doesn't distinguish the two.
pub(crate) fn hairpin_subtype(wedge: Wedge) -> Option<u8> {
    match wedge {
        Wedge::CrescendoStart => Some(0),
        Wedge::DiminuendoStart => Some(1),
        Wedge::Stop => None,
    }
}

pub(crate) fn wedge_from_hairpin_subtype(subtype: &str) -> Wedge {
    match subtype.trim() {
        "1" | "3" => Wedge::DiminuendoStart,
        _ => Wedge::CrescendoStart,
    }
}

/// How one [`Articulation`] is stored: most are an `<Articulation>` symbol
/// inside the `<Chord>`, but MuseScore models the brass falls/doits/plops/
/// scoops as a `<ChordLine>` (by numeric type) and breath marks/caesuras as
/// a `<Breath>` segment element after the chord.
pub(crate) enum ArticulationMark {
    Symbol(String),
    ChordLine(u8),
    Breath(&'static str),
}

pub(crate) fn articulation_to_mscx(articulation: &Articulation) -> ArticulationMark {
    let symbol = match articulation {
        Articulation::Accent => "articAccentAbove",
        Articulation::StrongAccent => "articMarcatoAbove",
        Articulation::Staccato => "articStaccatoAbove",
        Articulation::Tenuto => "articTenutoAbove",
        Articulation::DetachedLegato => "articTenutoStaccatoAbove",
        Articulation::Staccatissimo => "articStaccatissimoAbove",
        Articulation::Spiccato => "articStaccatissimoStrokeAbove",
        Articulation::Stress => "articStressAbove",
        Articulation::Unstress => "articUnstressAbove",
        Articulation::SoftAccent => "articSoftAccentAbove",
        // MuseScore's `ChordLineType`: NOTYPE, FALL, DOIT, PLOP, SCOOP.
        Articulation::Falloff => return ArticulationMark::ChordLine(1),
        Articulation::Doit => return ArticulationMark::ChordLine(2),
        Articulation::Plop => return ArticulationMark::ChordLine(3),
        Articulation::Scoop => return ArticulationMark::ChordLine(4),
        Articulation::BreathMark => return ArticulationMark::Breath("breathMarkComma"),
        Articulation::Caesura => return ArticulationMark::Breath("caesura"),
        Articulation::Other(symbol) => symbol,
    };
    ArticulationMark::Symbol(symbol.to_string())
}

pub(crate) fn articulation_from_chord_line(subtype: &str) -> Option<Articulation> {
    match subtype.trim() {
        "1" => Some(Articulation::Falloff),
        "2" => Some(Articulation::Doit),
        "3" => Some(Articulation::Plop),
        "4" => Some(Articulation::Scoop),
        _ => None,
    }
}

pub(crate) fn articulation_from_breath(symbol: &str) -> Articulation {
    if symbol.contains("aesura") {
        Articulation::Caesura
    } else {
        Articulation::BreathMark
    }
}

/// What an `<Articulation>`/`<Ornament>` symbol inside a `<Chord>` means.
/// MuseScore 3 stores ornaments (and, in old files, fermatas) as
/// articulation symbols too, so both elements share this classification.
pub(crate) enum ChordMark {
    Articulation(Articulation),
    Ornament(Ornament),
    Fermata,
}

pub(crate) fn chord_marks_from_symbol(symbol: &str) -> Vec<ChordMark> {
    use Articulation as A;
    let symbol = symbol.trim();
    if symbol.starts_with("fermata") {
        return vec![ChordMark::Fermata];
    }
    if symbol.starts_with("ornament") {
        return vec![ChordMark::Ornament(ornament_from_symbol(symbol))];
    }
    let base = symbol
        .strip_suffix("Above")
        .or_else(|| symbol.strip_suffix("Below"))
        .unwrap_or(symbol);
    let articulations = match base {
        "articAccent" => vec![A::Accent],
        "articMarcato" => vec![A::StrongAccent],
        "articStaccato" => vec![A::Staccato],
        "articTenuto" => vec![A::Tenuto],
        "articTenutoStaccato" => vec![A::DetachedLegato],
        "articStaccatissimo" | "articStaccatissimoWedge" => vec![A::Staccatissimo],
        "articStaccatissimoStroke" => vec![A::Spiccato],
        "articStress" => vec![A::Stress],
        "articUnstress" => vec![A::Unstress],
        "articSoftAccent" => vec![A::SoftAccent],
        "articAccentStaccato" => vec![A::Accent, A::Staccato],
        "articMarcatoStaccato" => vec![A::StrongAccent, A::Staccato],
        "articMarcatoTenuto" => vec![A::StrongAccent, A::Tenuto],
        "articTenutoAccent" => vec![A::Tenuto, A::Accent],
        _ => vec![A::Other(symbol.to_string())],
    };
    articulations
        .into_iter()
        .map(ChordMark::Articulation)
        .collect()
}

/// The SMuFL symbol MuseScore stores an ornament as (`<Ornament><subtype>`).
/// `None` for [`Ornament::Tremolo`] (a separate `<Tremolo>` element) and
/// [`Ornament::WavyLine`] (a `Trill` spanner, not supported). MuseScore
/// has no delayed turns, so those fall back to the plain turn symbols.
pub(crate) fn ornament_symbol(ornament: &Ornament) -> Option<&str> {
    Some(match ornament {
        Ornament::TrillMark => "ornamentTrill",
        Ornament::Turn | Ornament::DelayedTurn => "ornamentTurn",
        Ornament::InvertedTurn | Ornament::DelayedInvertedTurn => "ornamentTurnInverted",
        Ornament::VerticalTurn => "ornamentTurnUp",
        Ornament::InvertedVerticalTurn => "ornamentTurnUpS",
        Ornament::Shake => "ornamentShake3",
        Ornament::Mordent => "ornamentMordent",
        Ornament::InvertedMordent => "ornamentShortTrill",
        Ornament::Schleifer => "ornamentPrecompSlide",
        Ornament::Haydn => "ornamentHaydn",
        Ornament::Other(symbol) => symbol,
        Ornament::Tremolo(_) | Ornament::WavyLine => return None,
    })
}

pub(crate) fn ornament_from_symbol(symbol: &str) -> Ornament {
    match symbol {
        "ornamentTrill" => Ornament::TrillMark,
        "ornamentTurn" => Ornament::Turn,
        "ornamentTurnInverted" => Ornament::InvertedTurn,
        "ornamentTurnUp" => Ornament::VerticalTurn,
        "ornamentTurnUpS" => Ornament::InvertedVerticalTurn,
        "ornamentShake3" | "ornamentShakeMuffat1" => Ornament::Shake,
        "ornamentMordent" => Ornament::Mordent,
        // SMuFL renamed the inverted mordent; MuseScore 3 used the old name.
        "ornamentShortTrill" | "ornamentMordentInverted" => Ornament::InvertedMordent,
        "ornamentPrecompSlide" => Ornament::Schleifer,
        "ornamentHaydn" => Ornament::Haydn,
        other => Ornament::Other(other.to_string()),
    }
}

/// Single-chord tremolo subtype: `r8` (one stroke) .. `r64` (four).
pub(crate) fn tremolo_subtype(marks: u8) -> String {
    format!("r{}", 4u32 << marks.clamp(1, 4))
}

/// Inverse of [`tremolo_subtype`]; two-chord tremolos (`c8`..) are ignored.
pub(crate) fn tremolo_from_subtype(subtype: &str) -> Option<u8> {
    match subtype.trim() {
        "r8" => Some(1),
        "r16" => Some(2),
        "r32" => Some(3),
        "r64" => Some(4),
        _ => None,
    }
}

/// A `<BarLine>`'s contents. `span` is MuseScore's `spanFrom`/`spanTo`
/// preset for the "tick"/"short" styles.
pub(crate) struct MscxBarline {
    pub subtype: &'static str,
    pub span: Option<(i8, i8)>,
    pub visible: bool,
}

/// `None` for a regular barline, which MuseScore generates rather than
/// stores.
pub(crate) fn barline_to_mscx(kind: BarlineType) -> Option<MscxBarline> {
    let (subtype, span, visible) = match kind {
        BarlineType::Regular => return None,
        BarlineType::Dotted => ("dotted", None, true),
        BarlineType::Dashed => ("dashed", None, true),
        BarlineType::Heavy => ("heavy", None, true),
        BarlineType::LightLight => ("double", None, true),
        BarlineType::LightHeavy => ("end", None, true),
        BarlineType::HeavyLight => ("reverse-end", None, true),
        BarlineType::HeavyHeavy => ("double-heavy", None, true),
        BarlineType::Tick => ("normal", Some((-1, -7)), true),
        BarlineType::Short => ("normal", Some((2, -2)), true),
        BarlineType::NoBarline => ("normal", None, false),
    };
    Some(MscxBarline {
        subtype,
        span,
        visible,
    })
}

/// Inverse of [`barline_to_mscx`]. Repeat barline subtypes map to
/// `Regular` - the repeat itself is carried by `<startRepeat>`/`<endRepeat>`.
pub(crate) fn barline_from_mscx(
    subtype: &str,
    span_from: Option<i32>,
    visible: bool,
) -> BarlineType {
    if !visible {
        return BarlineType::NoBarline;
    }
    match subtype.trim() {
        "dotted" => BarlineType::Dotted,
        "dashed" => BarlineType::Dashed,
        "heavy" => BarlineType::Heavy,
        "double" => BarlineType::LightLight,
        "end" => BarlineType::LightHeavy,
        "reverse-end" => BarlineType::HeavyLight,
        "double-heavy" => BarlineType::HeavyHeavy,
        _ => match span_from {
            Some(from) if from < 0 => BarlineType::Tick,
            Some(from) if from > 0 => BarlineType::Short,
            _ => BarlineType::Regular,
        },
    }
}

/// How a navigation mark is stored: a `<Marker>` (identified by its
/// `<label>`) or a `<Jump>` (identified by its jump/play-until targets,
/// which reference marker labels).
pub(crate) enum NavigationMark {
    Marker {
        label: &'static str,
        text: &'static str,
    },
    Jump {
        text: &'static str,
        jump_to: &'static str,
        play_until: &'static str,
        continue_at: &'static str,
    },
}

pub(crate) fn navigation_to_mscx(navigation: Navigation) -> NavigationMark {
    let jump = |text, jump_to, play_until, continue_at| NavigationMark::Jump {
        text,
        jump_to,
        play_until,
        continue_at,
    };
    match navigation {
        Navigation::Segno => NavigationMark::Marker {
            label: "segno",
            text: "<sym>segno</sym>",
        },
        // MuseScore's labels are historical: the coda sign itself is
        // "codab" and the "To Coda" text is "coda".
        Navigation::Coda => NavigationMark::Marker {
            label: "codab",
            text: "<sym>coda</sym>",
        },
        Navigation::Fine => NavigationMark::Marker {
            label: "fine",
            text: "Fine",
        },
        Navigation::ToCoda => NavigationMark::Marker {
            label: "coda",
            text: "To Coda",
        },
        Navigation::DaCapo => jump("D.C.", "start", "end", ""),
        Navigation::DaCapoAlFine => jump("D.C. al Fine", "start", "fine", ""),
        Navigation::DaCapoAlCoda => jump("D.C. al Coda", "start", "coda", "codab"),
        Navigation::DalSegno => jump("D.S.", "segno", "end", ""),
        Navigation::DalSegnoAlFine => jump("D.S. al Fine", "segno", "fine", ""),
        Navigation::DalSegnoAlCoda => jump("D.S. al Coda", "segno", "coda", "codab"),
    }
}

/// A `<Marker>` by its `<label>`, falling back to its display text for
/// markers with a custom/empty label.
pub(crate) fn navigation_from_marker(label: &str, text: &str) -> Option<Navigation> {
    match label.trim() {
        "segno" | "varsegno" => Some(Navigation::Segno),
        "codab" | "varcoda" | "codetta" => Some(Navigation::Coda),
        "fine" => Some(Navigation::Fine),
        "coda" | "tocoda" | "tocodasym" => Some(Navigation::ToCoda),
        _ => match text.trim().to_ascii_lowercase().as_str() {
            "fine" => Some(Navigation::Fine),
            "to coda" => Some(Navigation::ToCoda),
            "segno" => Some(Navigation::Segno),
            "coda" => Some(Navigation::Coda),
            _ => None,
        },
    }
}

/// A `<Jump>` by where it jumps to (`start` = D.C., a segno = D.S.) and
/// where playback stops (`fine`/`coda`); falls back to the display text.
pub(crate) fn navigation_from_jump(
    jump_to: &str,
    play_until: &str,
    text: &str,
) -> Option<Navigation> {
    let text = text.trim().to_ascii_lowercase();
    let da_capo = match jump_to.trim() {
        "start" => true,
        "" => {
            if text.starts_with("d.c.") || text.starts_with("da capo") {
                true
            } else if text.starts_with("d.s.") || text.starts_with("dal segno") {
                false
            } else {
                return None;
            }
        }
        _ => false,
    };
    let until = match play_until.trim() {
        "" if text.ends_with("fine") => "fine",
        "" if text.ends_with("coda") => "coda",
        other => other,
    };
    Some(match (da_capo, until) {
        (true, "fine") => Navigation::DaCapoAlFine,
        (true, "coda" | "tocoda") => Navigation::DaCapoAlCoda,
        (true, _) => Navigation::DaCapo,
        (false, "fine") => Navigation::DalSegnoAlFine,
        (false, "coda" | "tocoda") => Navigation::DalSegnoAlCoda,
        (false, _) => Navigation::DalSegno,
    })
}

/// MuseScore's tonal pitch class (`<root>`/`<base>` of a `<Harmony>`):
/// position on the line of fifths with C = 14, G = 15, F = 13, C# = 21...
pub(crate) fn tpc_from_root(root: ChordRoot) -> i32 {
    let fifths = match root.step {
        Step::F => -1,
        Step::C => 0,
        Step::G => 1,
        Step::D => 2,
        Step::A => 3,
        Step::E => 4,
        Step::B => 5,
    };
    14 + fifths + 7 * root.alter as i32
}

pub(crate) fn root_from_tpc(tpc: i32) -> Option<ChordRoot> {
    let offset = tpc - 14;
    let alter = (offset + 1).div_euclid(7);
    if !(-2..=2).contains(&alter) {
        return None;
    }
    let step = match offset - 7 * alter {
        -1 => Step::F,
        0 => Step::C,
        1 => Step::G,
        2 => Step::D,
        3 => Step::A,
        4 => Step::E,
        _ => Step::B,
    };
    Some(ChordRoot::new(step, alter as i8))
}
