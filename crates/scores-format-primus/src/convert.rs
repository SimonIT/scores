//! Mappings between PriMus's values and the shared model: pitches (staff
//! step + alteration), durations (ticks + tuplet marker), symbol kinds,
//! clefs, barlines and navigation texts.

use primus_scores::measures::Barline as PriBarline;
use primus_scores::writer::Value;
use primus_scores::{Clef as PriClef, NoteSymbol};
use scores_core::{
    Articulation, BarlineType, Clef, Duration, Dynamic, Navigation, NoteValue, Ornament, Pitch,
    Step, Tuplet,
};

/// PriMus's time base: ticks per quarter note.
pub const TICKS_PER_QUARTER: u32 = 960;

const STEPS: [Step; 7] = [
    Step::C,
    Step::D,
    Step::E,
    Step::F,
    Step::G,
    Step::A,
    Step::B,
];

/// A staff step (`7 × (octave + 1) + letter`, middle C = 35) and
/// alteration as a pitch.
pub fn pitch_from_staff_step(staff_step: u8, alteration: i8) -> Pitch {
    Pitch::new(
        STEPS[usize::from(staff_step % 7)],
        alteration,
        (staff_step / 7) as i8 - 1,
    )
}

/// A pitch as a staff step and alteration; `None` below the staff-step
/// range (octave -1) or above it.
pub fn staff_step_from_pitch(pitch: Pitch) -> Option<(u8, i8)> {
    let letter = STEPS.iter().position(|&s| s == pitch.step)? as i32;
    let step = 7 * (i32::from(pitch.octave) + 1) + letter;
    u8::try_from(step)
        .ok()
        .map(|s| (s, pitch.alter.clamp(-2, 2)))
}

/// The written values PriMus and the model share, longest first, with
/// PriMus's value code and length in ticks.
const VALUES: [(NoteValue, u8, u32); 7] = [
    (NoteValue::Whole, 3, 3840),
    (NoteValue::Half, 4, 1920),
    (NoteValue::Quarter, 5, 960),
    (NoteValue::Eighth, 6, 480),
    (NoteValue::Sixteenth, 7, 240),
    (NoteValue::ThirtySecond, 8, 120),
    (NoteValue::SixtyFourth, 9, 60),
];

fn dotted_ticks(plain: u32, dots: u8) -> u32 {
    (0..=dots).map(|d| plain >> d).sum()
}

/// The plain (non-tuplet) value, with up to two dots, that lasts exactly
/// `ticks`.
fn exact_value(ticks: u32) -> Option<Duration> {
    VALUES.iter().find_map(|&(base, _, plain)| {
        (0..=2u8)
            .find(|&dots| dotted_ticks(plain, dots) == ticks)
            .map(|dots| Duration::dotted(base, dots))
    })
}

/// A record's duration from its played length in ticks and its tuplet
/// marker `(in_time_of, count)`. Lengths that no written value matches
/// (longas, breves, odd tuplets) are quantized to the nearest plain one.
pub fn duration_from_ticks(ticks: u32, tuplet_marker: Option<(u8, u8)>) -> Duration {
    if let Some((normal, actual)) = tuplet_marker.filter(|&(a, b)| a > 0 && b > 0 && a != b) {
        let written = ticks * u32::from(actual);
        if written.is_multiple_of(u32::from(normal))
            && let Some(mut d) = exact_value(written / u32::from(normal))
        {
            d.tuplet = Some(Tuplet { actual, normal });
            return d;
        }
    }
    exact_value(ticks).unwrap_or_else(|| {
        Duration::from_quarter_length_quantized(f64::from(ticks) / f64::from(TICKS_PER_QUARTER))
    })
}

/// A duration as a PriMus value, if PriMus can write it (at most two dots,
/// a tuplet that divides the value into whole ticks).
pub fn value_from_duration(duration: Duration) -> Option<Value> {
    let &(_, code, _) = VALUES.iter().find(|(base, ..)| *base == duration.base)?;
    let value = Value {
        code,
        dots: duration.dots,
        tuplet: duration.tuplet.map(|t| (t.normal, t.actual)),
    };
    (duration.dots <= 2 && value.ticks().is_some()).then_some(value)
}

/// Rests filling `ticks`, longest first; `None` if they can't fill it
/// exactly (a remainder shorter than a 64th).
pub fn rests_filling(mut ticks: u32) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    while ticks > 0 {
        let value = VALUES.iter().find_map(|&(_, code, plain)| {
            (0..=1u8)
                .rev()
                .find(|&dots| dotted_ticks(plain, dots) <= ticks)
                .map(|dots| Value {
                    code,
                    dots,
                    tuplet: None,
                })
        })?;
        ticks -= value.ticks()?;
        out.push(value);
    }
    Some(out)
}

/// Like [`rests_filling`], as durations.
pub fn rest_durations_filling(ticks: u32) -> Vec<Duration> {
    rests_filling(ticks)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.ticks())
        .filter_map(exact_value)
        .collect()
}

/// Symbol kinds (see [`NoteSymbol::name`]).
pub mod kind {
    pub const STACCATO: u8 = 0;
    pub const TENUTO: u8 = 1;
    pub const ACCENT: u8 = 2;
    pub const MARCATO: u8 = 3;
    pub const STACCATISSIMO: u8 = 5;
    pub const STACCATISSIMO_WEDGE: u8 = 6;
    pub const SCOOP: u8 = 19;
    pub const PLOP: u8 = 20;
    pub const FALL: u8 = 24;
    pub const DOIT: u8 = 27;
    pub const SHORT_TRILL: u8 = 35;
    pub const MORDENT: u8 = 36;
    pub const TURN: u8 = 37;
    pub const TRILL: u8 = 41;
    pub const TRILL_EXTENSIBLE: u8 = 44;
    pub const TRILL_WAVE: u8 = 46;
    pub const FERMATA: u8 = 53;
    pub const FERMATA_INVERTED: u8 = 54;
    pub const FERMATA_SHORT: u8 = 56;
    pub const FERMATA_LONG: u8 = 57;
    pub const SEGNO: u8 = 63;
    pub const BREATH_COMMA: u8 = 132;
    pub const BREATH_TICK: u8 = 133;
    pub const CAESURA: u8 = 135;
}

/// Dynamic symbol kinds, in PriMus's order.
const DYNAMICS: [(u8, Dynamic); 16] = [
    (112, Dynamic::F),
    (113, Dynamic::Ff),
    (114, Dynamic::Fff),
    (115, Dynamic::Ffff),
    (116, Dynamic::P),
    (117, Dynamic::Pp),
    (118, Dynamic::Ppp),
    (119, Dynamic::Pppp),
    (120, Dynamic::Mf),
    (121, Dynamic::Mp),
    (122, Dynamic::Fp),
    (123, Dynamic::Sfp),
    (124, Dynamic::Sfpp),
    (125, Dynamic::Fz),
    (126, Dynamic::Sf),
    (127, Dynamic::Sfz),
];

pub fn is_fermata(kind: u8) -> bool {
    matches!(
        kind,
        kind::FERMATA | kind::FERMATA_INVERTED | kind::FERMATA_SHORT | kind::FERMATA_LONG
    )
}

/// What a note's symbols say, in the model's terms.
#[derive(Default)]
pub struct Marks {
    pub dynamic: Option<Dynamic>,
    pub articulations: Vec<Articulation>,
    pub ornaments: Vec<Ornament>,
    pub fermata: bool,
    pub segno: bool,
}

pub fn marks_from_symbols(symbols: &[NoteSymbol]) -> Marks {
    let mut marks = Marks::default();
    let kinds: Vec<u8> = symbols.iter().map(|s| s.kind).collect();
    // Tenuto and staccato together are the model's detached legato.
    let detached = kinds.contains(&kind::TENUTO) && kinds.contains(&kind::STACCATO);
    for &k in &kinds {
        if let Some((_, d)) = DYNAMICS.iter().find(|(dk, _)| *dk == k) {
            marks.dynamic.get_or_insert_with(|| d.clone());
            continue;
        }
        let articulation = match k {
            kind::STACCATO | kind::TENUTO if detached => {
                if k == kind::TENUTO {
                    Some(Articulation::DetachedLegato)
                } else {
                    None
                }
            }
            kind::STACCATO => Some(Articulation::Staccato),
            kind::TENUTO => Some(Articulation::Tenuto),
            kind::ACCENT => Some(Articulation::Accent),
            kind::MARCATO => Some(Articulation::StrongAccent),
            kind::STACCATISSIMO => Some(Articulation::Staccatissimo),
            kind::STACCATISSIMO_WEDGE => Some(Articulation::Spiccato),
            kind::SCOOP => Some(Articulation::Scoop),
            kind::PLOP => Some(Articulation::Plop),
            kind::FALL => Some(Articulation::Falloff),
            kind::DOIT => Some(Articulation::Doit),
            kind::BREATH_COMMA | kind::BREATH_TICK => Some(Articulation::BreathMark),
            kind::CAESURA => Some(Articulation::Caesura),
            _ => None,
        };
        if let Some(a) = articulation {
            if !marks.articulations.contains(&a) {
                marks.articulations.push(a);
            }
            continue;
        }
        let ornament = match k {
            kind::TRILL | kind::TRILL_EXTENSIBLE => Some(Ornament::TrillMark),
            kind::TRILL_WAVE => Some(Ornament::WavyLine),
            kind::SHORT_TRILL => Some(Ornament::InvertedMordent),
            kind::MORDENT => Some(Ornament::Mordent),
            kind::TURN => Some(Ornament::Turn),
            _ => None,
        };
        if let Some(o) = ornament {
            if !marks.ornaments.contains(&o) {
                marks.ornaments.push(o);
            }
            continue;
        }
        marks.fermata |= is_fermata(k);
        marks.segno |= k == kind::SEGNO;
    }
    marks
}

pub fn dynamic_kind(dynamic: &Dynamic) -> Option<u8> {
    DYNAMICS.iter().find(|(_, d)| d == dynamic).map(|(k, _)| *k)
}

pub fn articulation_kinds(articulation: &Articulation) -> &'static [u8] {
    match articulation {
        Articulation::Staccato => &[kind::STACCATO],
        Articulation::Tenuto => &[kind::TENUTO],
        Articulation::DetachedLegato => &[kind::TENUTO, kind::STACCATO],
        Articulation::Accent => &[kind::ACCENT],
        Articulation::StrongAccent => &[kind::MARCATO],
        Articulation::Staccatissimo => &[kind::STACCATISSIMO],
        Articulation::Spiccato => &[kind::STACCATISSIMO_WEDGE],
        Articulation::Scoop => &[kind::SCOOP],
        Articulation::Plop => &[kind::PLOP],
        Articulation::Falloff => &[kind::FALL],
        Articulation::Doit => &[kind::DOIT],
        Articulation::BreathMark => &[kind::BREATH_COMMA],
        Articulation::Caesura => &[kind::CAESURA],
        _ => &[],
    }
}

pub fn ornament_kind(ornament: &Ornament) -> Option<u8> {
    Some(match ornament {
        Ornament::TrillMark => kind::TRILL,
        Ornament::WavyLine => kind::TRILL_WAVE,
        Ornament::InvertedMordent | Ornament::Shake => kind::SHORT_TRILL,
        Ornament::Mordent => kind::MORDENT,
        Ornament::Turn => kind::TURN,
        _ => return None,
    })
}

pub fn clef_from_primus(clef: PriClef) -> Clef {
    match clef {
        PriClef::Bass | PriClef::BassOctaveUp | PriClef::BassOctaveDown | PriClef::Baritone => {
            Clef::Bass
        }
        PriClef::Alto | PriClef::Soprano | PriClef::MezzoSoprano => Clef::Alto,
        PriClef::Tenor => Clef::Tenor,
        PriClef::Percussion => Clef::Percussion,
        _ => Clef::Treble,
    }
}

pub fn clef_to_primus(clef: Clef) -> PriClef {
    match clef {
        Clef::Treble => PriClef::Treble,
        Clef::Bass => PriClef::Bass,
        Clef::Alto => PriClef::Alto,
        Clef::Tenor => PriClef::Tenor,
        Clef::Percussion => PriClef::Percussion,
    }
}

/// A closing barline's style, ignoring repeat signs (those are the
/// model's repeat flags).
pub fn barline_kind_from_primus(barline: PriBarline) -> BarlineType {
    match barline {
        PriBarline::Double | PriBarline::ThinHeavyThin => BarlineType::LightLight,
        PriBarline::HeavyThin => BarlineType::HeavyLight,
        PriBarline::Final => BarlineType::LightHeavy,
        PriBarline::Invisible => BarlineType::NoBarline,
        PriBarline::Dotted => BarlineType::Dotted,
        PriBarline::Dashed => BarlineType::Dashed,
        _ => BarlineType::Regular,
    }
}

pub fn barline_kind_to_primus(kind: BarlineType) -> PriBarline {
    match kind {
        BarlineType::LightLight | BarlineType::HeavyHeavy => PriBarline::Double,
        BarlineType::HeavyLight => PriBarline::HeavyThin,
        BarlineType::LightHeavy | BarlineType::Heavy => PriBarline::Final,
        BarlineType::NoBarline => PriBarline::Invisible,
        BarlineType::Dotted => PriBarline::Dotted,
        BarlineType::Dashed => PriBarline::Dashed,
        _ => PriBarline::Normal,
    }
}

/// Navigation marks are free measure texts in PriMus; this is the text
/// written for each (the segno is a symbol when a note carries it).
pub fn navigation_text(navigation: Navigation) -> &'static str {
    match navigation {
        Navigation::Segno => "Segno",
        Navigation::Coda => "Coda",
        Navigation::DaCapo => "D.C.",
        Navigation::DaCapoAlFine => "D.C. al Fine",
        Navigation::DaCapoAlCoda => "D.C. al Coda",
        Navigation::DalSegno => "D.S.",
        Navigation::DalSegnoAlFine => "D.S. al Fine",
        Navigation::DalSegnoAlCoda => "D.S. al Coda",
        Navigation::Fine => "Fine",
        Navigation::ToCoda => "To Coda",
    }
}

/// The navigation mark a measure text reads as, if any.
pub fn navigation_from_text(text: &str) -> Option<Navigation> {
    let t: String = text
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let (jump, rest) = if let Some(rest) = t.strip_prefix("d.c.").or(t.strip_prefix("dacapo")) {
        (Some(true), rest)
    } else if let Some(rest) = t.strip_prefix("d.s.").or(t.strip_prefix("dalsegno")) {
        (Some(false), rest)
    } else {
        (None, t.as_str())
    };
    Some(match (jump, rest) {
        (Some(true), "") => Navigation::DaCapo,
        (Some(true), "alfine") => Navigation::DaCapoAlFine,
        (Some(true), "alcoda") => Navigation::DaCapoAlCoda,
        (Some(false), "") => Navigation::DalSegno,
        (Some(false), "alfine") => Navigation::DalSegnoAlFine,
        (Some(false), "alcoda") => Navigation::DalSegnoAlCoda,
        (None, "fine") => Navigation::Fine,
        (None, "tocoda" | "zurcoda") => Navigation::ToCoda,
        (None, "coda") => Navigation::Coda,
        (None, "segno") => Navigation::Segno,
        _ => return None,
    })
}

/// An ending's pass numbers from its label ("1.", "2.", "1.-3.", "1., 2.").
pub fn ending_numbers(label: &str) -> Vec<u8> {
    let mut numbers = Vec::new();
    let mut current: Option<u32> = None;
    for c in label.chars().chain(std::iter::once(' ')) {
        match c.to_digit(10) {
            Some(d) => current = Some(current.unwrap_or(0) * 10 + d),
            None => {
                if let Some(n) = current.take().and_then(|n| u8::try_from(n).ok()) {
                    numbers.push(n);
                }
            }
        }
    }
    if numbers.len() == 2 && label.contains('-') && numbers[0] < numbers[1] {
        numbers = (numbers[0]..=numbers[1]).collect();
    }
    numbers
}

/// An ending's label, the way PriMus labels them: "1.", "1.-3.", "1., 3.".
pub fn ending_label(numbers: &[u8]) -> String {
    let contiguous = numbers.windows(2).all(|w| w[1] == w[0] + 1);
    match numbers {
        [] => String::new(),
        [first, .., last] if contiguous && numbers.len() > 2 => format!("{first}.-{last}."),
        _ => numbers
            .iter()
            .map(|n| format!("{n}."))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// `text` with every character Windows-1252 can't encode replaced by `?`
/// (PriMus's texts are Windows-1252).
pub fn windows_1252(text: &str) -> String {
    const EXTRAS: &str = "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ";
    text.chars()
        .map(|c| {
            let code = c as u32;
            if code < 0x80 || (0xA0..=0xFF).contains(&code) || EXTRAS.contains(c) {
                c
            } else {
                '?'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staff_steps_round_trip() {
        let c4 = pitch_from_staff_step(35, 0);
        assert_eq!(c4, Pitch::new(Step::C, 0, 4));
        assert_eq!(
            staff_step_from_pitch(Pitch::new(Step::A, -1, 4)),
            Some((40, -1))
        );
    }

    #[test]
    fn durations_from_ticks() {
        assert_eq!(
            duration_from_ticks(1440, None),
            Duration::dotted(NoteValue::Quarter, 1)
        );
        let triplet = duration_from_ticks(320, Some((2, 3)));
        assert_eq!(triplet.base, NoteValue::Eighth);
        assert_eq!(
            triplet.tuplet,
            Some(Tuplet {
                actual: 3,
                normal: 2
            })
        );
        let quintuplet = duration_from_ticks(192, Some((4, 5)));
        assert_eq!(quintuplet.base, NoteValue::Sixteenth);
    }

    #[test]
    fn rests_fill_odd_lengths() {
        let rests = rests_filling(3840 + 960).unwrap();
        assert_eq!(rests.iter().map(|v| v.ticks().unwrap()).sum::<u32>(), 4800);
        assert!(rests_filling(30).is_none());
    }

    #[test]
    fn ending_labels_round_trip() {
        assert_eq!(ending_numbers("1.-3."), [1, 2, 3]);
        assert_eq!(ending_numbers("1., 2."), [1, 2]);
        assert_eq!(ending_label(&[1, 2, 3]), "1.-3.");
        assert_eq!(ending_label(&[2]), "2.");
        assert_eq!(ending_label(&[1, 2]), "1., 2.");
    }

    #[test]
    fn navigation_texts_round_trip() {
        for nav in [
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
        ] {
            assert_eq!(navigation_from_text(navigation_text(nav)), Some(nav));
        }
        assert_eq!(navigation_from_text("dolce"), None);
    }
}
