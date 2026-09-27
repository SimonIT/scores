//! Chord symbols (lead-sheet harmony such as `Cmaj7` or `F#m7b5/E`).
//!
//! Many formats only store a chord symbol as display text (ABC annotations,
//! MEI `<harm>`, MuseScore `<Harmony>`, Guitar Pro chord names, ChordPro), so
//! [`ChordSymbol::parse`] and the [`Display`](std::fmt::Display) impl give all
//! of them one shared text form rather than a separate parser per plugin.

use crate::model::Step;
use std::fmt;

/// A chord symbol: root, chord quality and an optional different bass note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChordSymbol {
    pub root: ChordRoot,
    pub kind: ChordKind,
    /// The bass note of a slash chord (`C/G`), if it differs from the root.
    pub bass: Option<ChordRoot>,
}

/// A pitch class used as a chord root or bass: step plus semitone alteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordRoot {
    pub step: Step,
    /// Semitone alteration: -2 (double flat) ..= 2 (double sharp).
    pub alter: i8,
}

impl ChordRoot {
    pub fn new(step: Step, alter: i8) -> Self {
        Self { step, alter }
    }

    /// Semitones above C, wrapped into `0..12`.
    pub fn pitch_class(&self) -> u8 {
        (self.step.semitone() as i32 + self.alter as i32).rem_euclid(12) as u8
    }
}

/// Chord quality, modeled on MusicXML's `kind-value` list (the most complete
/// shared vocabulary between formats). `Other` holds a suffix no variant
/// covers (e.g. `7b9`, `add9`) verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordKind {
    Major,
    Minor,
    Augmented,
    Diminished,
    Dominant,
    MajorSeventh,
    MinorSeventh,
    DiminishedSeventh,
    AugmentedSeventh,
    HalfDiminished,
    MajorMinor,
    MajorSixth,
    MinorSixth,
    DominantNinth,
    MajorNinth,
    MinorNinth,
    Dominant11th,
    Major11th,
    Minor11th,
    Dominant13th,
    Major13th,
    Minor13th,
    SuspendedSecond,
    SuspendedFourth,
    Power,
    /// "N.C." - explicitly no chord. The root is meaningless for this kind.
    NoChord,
    Other(String),
}

impl ChordKind {
    /// The suffix written after the root in [`ChordSymbol`]'s text form.
    pub fn suffix(&self) -> &str {
        match self {
            ChordKind::Major => "",
            ChordKind::Minor => "m",
            ChordKind::Augmented => "aug",
            ChordKind::Diminished => "dim",
            ChordKind::Dominant => "7",
            ChordKind::MajorSeventh => "maj7",
            ChordKind::MinorSeventh => "m7",
            ChordKind::DiminishedSeventh => "dim7",
            ChordKind::AugmentedSeventh => "aug7",
            ChordKind::HalfDiminished => "m7b5",
            ChordKind::MajorMinor => "m(maj7)",
            ChordKind::MajorSixth => "6",
            ChordKind::MinorSixth => "m6",
            ChordKind::DominantNinth => "9",
            ChordKind::MajorNinth => "maj9",
            ChordKind::MinorNinth => "m9",
            ChordKind::Dominant11th => "11",
            ChordKind::Major11th => "maj11",
            ChordKind::Minor11th => "m11",
            ChordKind::Dominant13th => "13",
            ChordKind::Major13th => "maj13",
            ChordKind::Minor13th => "m13",
            ChordKind::SuspendedSecond => "sus2",
            ChordKind::SuspendedFourth => "sus4",
            ChordKind::Power => "5",
            ChordKind::NoChord => "N.C.",
            ChordKind::Other(suffix) => suffix,
        }
    }

    /// Recognizes the common spellings of a chord suffix (`-7`, `min7`,
    /// `m7` all mean [`ChordKind::MinorSeventh`]); anything else becomes
    /// [`ChordKind::Other`].
    pub fn from_suffix(suffix: &str) -> ChordKind {
        match suffix {
            "" | "maj" | "M" | "ma" => ChordKind::Major,
            "m" | "min" | "mi" | "-" => ChordKind::Minor,
            "aug" | "+" => ChordKind::Augmented,
            "dim" | "o" | "°" => ChordKind::Diminished,
            "7" | "dom7" => ChordKind::Dominant,
            "maj7" | "M7" | "ma7" | "Δ" | "Δ7" | "j7" => ChordKind::MajorSeventh,
            "m7" | "min7" | "mi7" | "-7" => ChordKind::MinorSeventh,
            "dim7" | "o7" | "°7" => ChordKind::DiminishedSeventh,
            "aug7" | "+7" | "7#5" | "7+5" | "7+" => ChordKind::AugmentedSeventh,
            "m7b5" | "min7b5" | "mi7b5" | "-7b5" | "ø" | "ø7" => ChordKind::HalfDiminished,
            "m(maj7)" | "mmaj7" | "mMaj7" | "mM7" | "minmaj7" | "-maj7" | "-Δ7" | "mΔ7" => {
                ChordKind::MajorMinor
            }
            "6" | "maj6" | "M6" => ChordKind::MajorSixth,
            "m6" | "min6" | "mi6" | "-6" => ChordKind::MinorSixth,
            "9" => ChordKind::DominantNinth,
            "maj9" | "M9" | "ma9" | "Δ9" => ChordKind::MajorNinth,
            "m9" | "min9" | "mi9" | "-9" => ChordKind::MinorNinth,
            "11" => ChordKind::Dominant11th,
            "maj11" | "M11" | "ma11" => ChordKind::Major11th,
            "m11" | "min11" | "mi11" | "-11" => ChordKind::Minor11th,
            "13" => ChordKind::Dominant13th,
            "maj13" | "M13" | "ma13" => ChordKind::Major13th,
            "m13" | "min13" | "mi13" | "-13" => ChordKind::Minor13th,
            "sus2" => ChordKind::SuspendedSecond,
            "sus4" | "sus" => ChordKind::SuspendedFourth,
            "5" => ChordKind::Power,
            other => ChordKind::Other(other.to_string()),
        }
    }

    /// Chord tones as semitones above the root, root first. [`ChordKind::Other`]
    /// and [`ChordKind::NoChord`] have no known tones beyond the root/nothing.
    pub fn intervals(&self) -> &'static [u8] {
        match self {
            ChordKind::Major => &[0, 4, 7],
            ChordKind::Minor => &[0, 3, 7],
            ChordKind::Augmented => &[0, 4, 8],
            ChordKind::Diminished => &[0, 3, 6],
            ChordKind::Dominant => &[0, 4, 7, 10],
            ChordKind::MajorSeventh => &[0, 4, 7, 11],
            ChordKind::MinorSeventh => &[0, 3, 7, 10],
            ChordKind::DiminishedSeventh => &[0, 3, 6, 9],
            ChordKind::AugmentedSeventh => &[0, 4, 8, 10],
            ChordKind::HalfDiminished => &[0, 3, 6, 10],
            ChordKind::MajorMinor => &[0, 3, 7, 11],
            ChordKind::MajorSixth => &[0, 4, 7, 9],
            ChordKind::MinorSixth => &[0, 3, 7, 9],
            ChordKind::DominantNinth => &[0, 4, 7, 10, 14],
            ChordKind::MajorNinth => &[0, 4, 7, 11, 14],
            ChordKind::MinorNinth => &[0, 3, 7, 10, 14],
            ChordKind::Dominant11th => &[0, 4, 7, 10, 14, 17],
            ChordKind::Major11th => &[0, 4, 7, 11, 14, 17],
            ChordKind::Minor11th => &[0, 3, 7, 10, 14, 17],
            ChordKind::Dominant13th => &[0, 4, 7, 10, 14, 21],
            ChordKind::Major13th => &[0, 4, 7, 11, 14, 21],
            ChordKind::Minor13th => &[0, 3, 7, 10, 14, 21],
            ChordKind::SuspendedSecond => &[0, 2, 7],
            ChordKind::SuspendedFourth => &[0, 5, 7],
            ChordKind::Power => &[0, 7],
            ChordKind::NoChord => &[],
            ChordKind::Other(_) => &[0],
        }
    }
}

impl ChordSymbol {
    pub fn new(root: ChordRoot, kind: ChordKind) -> Self {
        Self {
            root,
            kind,
            bass: None,
        }
    }

    /// An explicit "no chord" (N.C.) marking.
    pub fn no_chord() -> Self {
        Self::new(ChordRoot::new(Step::C, 0), ChordKind::NoChord)
    }

    /// Parses a chord symbol's text form, e.g. `C`, `F#m7`, `Bbmaj7/D`,
    /// `N.C.`. Returns `None` if the text doesn't start with a root note
    /// name (`A`-`G`), so arbitrary annotations aren't misread as chords.
    pub fn parse(text: &str) -> Option<ChordSymbol> {
        let text = text.trim();
        if matches!(text, "N.C." | "NC" | "N.C" | "n.c.") {
            return Some(Self::no_chord());
        }
        let (root, rest) = parse_root(text)?;
        let (suffix, bass) = match rest.rsplit_once('/') {
            Some((suffix, bass_text)) => match parse_root(bass_text) {
                Some((bass, "")) => (suffix, Some(bass)),
                _ => (rest, None),
            },
            None => (rest, None),
        };
        // A suffix that doesn't look like chord-quality text (spaces, a
        // lowercase word such as "rit.") means this wasn't a chord symbol.
        if suffix.contains(char::is_whitespace) {
            return None;
        }
        Some(ChordSymbol {
            root,
            kind: ChordKind::from_suffix(suffix),
            bass,
        })
    }
}

fn parse_root(text: &str) -> Option<(ChordRoot, &str)> {
    let mut chars = text.char_indices();
    let (_, letter) = chars.next()?;
    let step = match letter {
        'C' => Step::C,
        'D' => Step::D,
        'E' => Step::E,
        'F' => Step::F,
        'G' => Step::G,
        'A' => Step::A,
        'B' => Step::B,
        _ => return None,
    };
    let mut alter = 0i8;
    let mut end = letter.len_utf8();
    for (i, c) in chars {
        match c {
            '#' | '♯' => alter += 1,
            'b' | '♭' => alter -= 1,
            _ => break,
        }
        end = i + c.len_utf8();
    }
    Some((ChordRoot::new(step, alter.clamp(-2, 2)), &text[end..]))
}

impl fmt::Display for ChordRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let letter = match self.step {
            Step::C => 'C',
            Step::D => 'D',
            Step::E => 'E',
            Step::F => 'F',
            Step::G => 'G',
            Step::A => 'A',
            Step::B => 'B',
        };
        write!(f, "{letter}")?;
        let accidental = if self.alter >= 0 { "#" } else { "b" };
        for _ in 0..self.alter.unsigned_abs() {
            f.write_str(accidental)?;
        }
        Ok(())
    }
}

impl fmt::Display for ChordSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == ChordKind::NoChord {
            return f.write_str("N.C.");
        }
        write!(f, "{}{}", self.root, self.kind.suffix())?;
        if let Some(bass) = self.bass {
            write!(f, "/{bass}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_spellings() {
        let c = ChordSymbol::parse("Bbmaj7/D").unwrap();
        assert_eq!(c.root, ChordRoot::new(Step::B, -1));
        assert_eq!(c.kind, ChordKind::MajorSeventh);
        assert_eq!(c.bass, Some(ChordRoot::new(Step::D, 0)));

        assert_eq!(
            ChordSymbol::parse("F#-7").unwrap().kind,
            ChordKind::MinorSeventh
        );
        assert_eq!(
            ChordSymbol::parse("Cø").unwrap().kind,
            ChordKind::HalfDiminished
        );
        assert_eq!(ChordSymbol::parse("N.C.").unwrap().kind, ChordKind::NoChord);
        assert_eq!(
            ChordSymbol::parse("G7b9").unwrap().kind,
            ChordKind::Other("7b9".into())
        );
    }

    #[test]
    fn rejects_non_chord_text() {
        assert!(ChordSymbol::parse("rit.").is_none());
        assert!(ChordSymbol::parse("").is_none());
        assert!(ChordSymbol::parse("A tempo").is_none());
    }

    #[test]
    fn text_form_round_trips() {
        for text in [
            "C",
            "Am",
            "F#m7b5",
            "Ebmaj7/G",
            "Dsus4",
            "N.C.",
            "G7b9",
            "Bbm(maj7)",
        ] {
            let parsed = ChordSymbol::parse(text).unwrap();
            assert_eq!(parsed.to_string(), text);
            assert_eq!(ChordSymbol::parse(&parsed.to_string()), Some(parsed));
        }
    }
}
