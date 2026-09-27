//! Format-agnostic in-memory representation of a music score.
//!
//! Every format plugin reads into and writes out of this model, so it only
//! captures what is reasonably common across notation and performance
//! formats: pitched/rested events grouped into voices and measures, with
//! enough metadata (key, time signature, clef, tempo, dynamics,
//! articulations, ornaments, chord symbols, repeats) to round-trip between e.g. MusicXML,
//! MEI, ABC, MIDI and Guitar Pro. Not every format can carry every part of
//! this model - each plugin documents which parts it actually reads/writes.

use crate::chord_symbol::ChordSymbol;

/// A complete score: metadata plus one part per instrument/voice staff.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    pub title: Option<String>,
    pub composer: Option<String>,
    pub parts: Vec<Part>,
}

/// A single instrument/staff, made up of consecutive measures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Part {
    pub id: String,
    pub name: Option<String>,
    pub measures: Vec<Measure>,
}

/// One measure (bar) of music: one or more simultaneous [`Voice`]s, plus
/// whatever changes (attributes, barline, navigation marks) take effect at
/// this measure.
///
/// `attributes` only needs to be set on the measure where a value changes
/// (or the first measure, to establish the initial value) - readers/writers
/// that need the "current" value for every measure should track the last
/// seen attributes as they walk through a part.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measure {
    pub number: u32,
    pub attributes: MeasureAttributes,
    pub voices: Vec<Voice>,
    pub barline: Barline,
    /// Navigation marks in this measure, in written order. Signs
    /// ([`Navigation::Segno`], [`Navigation::Coda`]) mark the measure's start;
    /// jumps, [`Navigation::Fine`] and [`Navigation::ToCoda`] take effect at
    /// its end.
    pub navigation: Vec<Navigation>,
}

impl Measure {
    /// Convenience for the common single-voice case.
    pub fn single_voice(number: u32, attributes: MeasureAttributes, events: Vec<Event>) -> Self {
        Self {
            number,
            attributes,
            voices: vec![Voice { number: 1, events }],
            barline: Barline::default(),
            navigation: Vec::new(),
        }
    }
}

/// An independent rhythmic stream within a measure. Multiple voices in the
/// same measure sound simultaneously; a single voice's own events are
/// sequential in time (chords handle simultaneity within one voice).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Voice {
    pub number: u8,
    pub events: Vec<Event>,
}

/// Notation attributes that can change at the start of a measure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeasureAttributes {
    pub time_signature: Option<TimeSignature>,
    pub key_signature: Option<KeySignature>,
    pub clef: Option<Clef>,
    pub tempo: Option<Tempo>,
}

/// A single musical event occupying part of a voice's timeline.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Note(Note),
    /// Two or more notes sounding together for the same duration.
    Chord(Vec<Note>),
    Rest(Rest),
}

impl Event {
    pub fn duration(&self) -> Duration {
        match self {
            Event::Note(n) => n.duration,
            Event::Chord(notes) => notes.first().map(|n| n.duration).unwrap_or_default(),
            Event::Rest(r) => r.duration,
        }
    }

    /// The chord symbol attached to this event (for a chord: its first note's).
    pub fn chord_symbol(&self) -> Option<&ChordSymbol> {
        match self {
            Event::Note(n) => n.chord_symbol.as_ref(),
            Event::Chord(notes) => notes.first().and_then(|n| n.chord_symbol.as_ref()),
            Event::Rest(r) => r.chord_symbol.as_ref(),
        }
    }

    /// Attaches `symbol` to this event (for a chord: to its first note).
    pub fn set_chord_symbol(&mut self, symbol: Option<ChordSymbol>) {
        match self {
            Event::Note(n) => n.chord_symbol = symbol,
            Event::Chord(notes) => {
                if let Some(first) = notes.first_mut() {
                    first.chord_symbol = symbol;
                }
            }
            Event::Rest(r) => r.chord_symbol = symbol,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub pitch: Pitch,
    pub duration: Duration,
    pub tie: Tie,
    /// A grace note borrows no time from the measure: readers/writers that
    /// sum event durations to track a time cursor must skip it entirely.
    pub grace: bool,
    pub fermata: bool,
    pub articulations: Vec<Articulation>,
    pub ornaments: Vec<Ornament>,
    /// A dynamic marking (mf, f, sfz, ...) beginning at this note.
    pub dynamic: Option<Dynamic>,
    /// A crescendo/diminuendo hairpin starting or stopping at this note.
    pub wedge: Option<Wedge>,
    /// A chord symbol (lead-sheet harmony, e.g. `Cmaj7`) written above this
    /// note. In an [`Event::Chord`] it is carried by the first note only.
    pub chord_symbol: Option<ChordSymbol>,
}

impl Note {
    /// A plain note with no articulations/ornaments/dynamics - the common case.
    pub fn new(pitch: Pitch, duration: Duration, tie: Tie) -> Self {
        Self {
            pitch,
            duration,
            tie,
            grace: false,
            fermata: false,
            articulations: Vec::new(),
            ornaments: Vec::new(),
            dynamic: None,
            wedge: None,
            chord_symbol: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rest {
    pub duration: Duration,
    pub fermata: bool,
    /// A chord symbol written above this rest (common in lead sheets).
    pub chord_symbol: Option<ChordSymbol>,
}

impl Rest {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            fermata: false,
            chord_symbol: None,
        }
    }
}

/// Whether (and how) a note is tied to its neighbour(s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tie {
    #[default]
    None,
    Start,
    Stop,
    /// Tied to both the previous and the next note.
    StartStop,
}

/// A dynamic marking. `Other` mirrors a format's own open-ended
/// "other-dynamics"/free-text extension point, not an escape hatch for
/// unhandled common cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dynamic {
    P,
    Pp,
    Ppp,
    Pppp,
    Ppppp,
    Pppppp,
    F,
    Ff,
    Fff,
    Ffff,
    Fffff,
    Ffffff,
    Mp,
    Mf,
    Sf,
    Sfp,
    Sfpp,
    Fp,
    Rf,
    Rfz,
    Sfz,
    Sffz,
    Fz,
    N,
    Pf,
    Sfzp,
    Other(String),
}

/// A crescendo/diminuendo hairpin marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wedge {
    CrescendoStart,
    DiminuendoStart,
    Stop,
}

/// An articulation/accent mark on a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Articulation {
    Accent,
    StrongAccent,
    Staccato,
    Tenuto,
    DetachedLegato,
    Staccatissimo,
    Spiccato,
    Scoop,
    Plop,
    Doit,
    Falloff,
    BreathMark,
    Caesura,
    Stress,
    Unstress,
    SoftAccent,
    Other(String),
}

/// An ornament on a note. `Tremolo` carries the number of tremolo marks
/// (slashes/beams).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ornament {
    TrillMark,
    Turn,
    DelayedTurn,
    InvertedTurn,
    DelayedInvertedTurn,
    VerticalTurn,
    InvertedVerticalTurn,
    Shake,
    WavyLine,
    Mordent,
    InvertedMordent,
    Schleifer,
    Tremolo(u8),
    Haydn,
    Other(String),
}

/// A pitch in scientific pitch notation (`octave` 4 contains middle C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pitch {
    pub step: Step,
    /// Semitone alteration: -2 (double flat) ..= 2 (double sharp).
    pub alter: i8,
    pub octave: i8,
}

impl Pitch {
    pub fn new(step: Step, alter: i8, octave: i8) -> Self {
        Self {
            step,
            alter,
            octave,
        }
    }

    /// Converts to a MIDI note number (middle C / C4 = 60).
    pub fn to_midi_number(&self) -> i32 {
        (self.octave as i32 + 1) * 12 + self.step.semitone() as i32 + self.alter as i32
    }

    /// Builds the pitch closest to `midi_number`, spelled using sharps
    /// (or flats, if `prefer_flats` is set) rather than double accidentals.
    pub fn from_midi_number(midi_number: i32, prefer_flats: bool) -> Self {
        let octave = midi_number.div_euclid(12) - 1;
        let semitone = midi_number.rem_euclid(12) as u8;
        let (step, alter) = if prefer_flats {
            FLAT_SPELLING[semitone as usize]
        } else {
            SHARP_SPELLING[semitone as usize]
        };
        Self {
            step,
            alter,
            octave: octave as i8,
        }
    }
}

/// Diatonic step name, without accidental or octave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    C,
    D,
    E,
    F,
    G,
    A,
    B,
}

impl Step {
    /// Semitones above C, ignoring accidentals (C=0 .. B=11).
    pub fn semitone(&self) -> u8 {
        match self {
            Step::C => 0,
            Step::D => 2,
            Step::E => 4,
            Step::F => 5,
            Step::G => 7,
            Step::A => 9,
            Step::B => 11,
        }
    }
}

const SHARP_SPELLING: [(Step, i8); 12] = [
    (Step::C, 0),
    (Step::C, 1),
    (Step::D, 0),
    (Step::D, 1),
    (Step::E, 0),
    (Step::F, 0),
    (Step::F, 1),
    (Step::G, 0),
    (Step::G, 1),
    (Step::A, 0),
    (Step::A, 1),
    (Step::B, 0),
];

const FLAT_SPELLING: [(Step, i8); 12] = [
    (Step::C, 0),
    (Step::D, -1),
    (Step::D, 0),
    (Step::E, -1),
    (Step::E, 0),
    (Step::F, 0),
    (Step::G, -1),
    (Step::G, 0),
    (Step::A, -1),
    (Step::A, 0),
    (Step::B, -1),
    (Step::B, 0),
];

/// The written note value a [`Duration`] is based on, before dots/tuplets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteValue {
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
}

impl NoteValue {
    /// All values, from longest to shortest - useful for quantization.
    pub const ALL: [NoteValue; 7] = [
        NoteValue::Whole,
        NoteValue::Half,
        NoteValue::Quarter,
        NoteValue::Eighth,
        NoteValue::Sixteenth,
        NoteValue::ThirtySecond,
        NoteValue::SixtyFourth,
    ];

    /// Length in quarter notes (a quarter note is `1.0`).
    pub fn quarter_length(&self) -> f64 {
        match self {
            NoteValue::Whole => 4.0,
            NoteValue::Half => 2.0,
            NoteValue::Quarter => 1.0,
            NoteValue::Eighth => 0.5,
            NoteValue::Sixteenth => 0.25,
            NoteValue::ThirtySecond => 0.125,
            NoteValue::SixtyFourth => 0.0625,
        }
    }
}

/// An irregular grouping such as a triplet: `actual` notes played in the
/// time of `normal` notes (e.g. a triplet eighth is `actual: 3, normal: 2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuplet {
    pub actual: u8,
    pub normal: u8,
}

/// A rhythmic duration: a base note value, optional augmentation dots and an
/// optional tuplet ratio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duration {
    pub base: NoteValue,
    pub dots: u8,
    pub tuplet: Option<Tuplet>,
}

impl Default for Duration {
    fn default() -> Self {
        Duration::new(NoteValue::Quarter)
    }
}

impl Duration {
    pub fn new(base: NoteValue) -> Self {
        Self {
            base,
            dots: 0,
            tuplet: None,
        }
    }

    pub fn dotted(base: NoteValue, dots: u8) -> Self {
        Self {
            base,
            dots,
            tuplet: None,
        }
    }

    /// Length in quarter notes, taking dots and tuplet ratio into account.
    pub fn quarter_length(&self) -> f64 {
        let mut length = self.base.quarter_length();
        let mut addend = length;
        for _ in 0..self.dots {
            addend /= 2.0;
            length += addend;
        }
        if let Some(tuplet) = self.tuplet {
            length *= tuplet.normal as f64 / tuplet.actual as f64;
        }
        length
    }

    /// Finds the plain (non-tuplet) duration - up to 2 augmentation dots -
    /// whose length in quarter notes is closest to `quarter_length`.
    ///
    /// Used by formats like MIDI that only carry an absolute time span and
    /// need it re-quantized into a notated duration.
    pub fn from_quarter_length_quantized(quarter_length: f64) -> Self {
        let mut best = Duration::new(NoteValue::Quarter);
        let mut best_diff = f64::MAX;
        for base in NoteValue::ALL {
            for dots in 0..=2u8 {
                let candidate = Duration::dotted(base, dots);
                let diff = (candidate.quarter_length() - quarter_length).abs();
                if diff < best_diff {
                    best_diff = diff;
                    best = candidate;
                }
            }
        }
        best
    }
}

/// Clef in effect for a staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clef {
    #[default]
    Treble,
    Bass,
    Alto,
    Tenor,
    Percussion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub beats: u8,
    pub beat_type: u8,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self {
            beats: 4,
            beat_type: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Major,
    Minor,
}

/// A key signature expressed as a position on the circle of fifths:
/// `fifths` is the number of sharps (positive) or flats (negative), 0 = C
/// major / A minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySignature {
    pub fifths: i8,
    pub mode: Mode,
}

impl Default for KeySignature {
    fn default() -> Self {
        Self {
            fifths: 0,
            mode: Mode::Major,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tempo {
    /// Quarter notes per minute.
    pub bpm: f32,
}

impl Default for Tempo {
    fn default() -> Self {
        Self { bpm: 120.0 }
    }
}

/// The visual style of a barline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarlineType {
    #[default]
    Regular,
    Dotted,
    Dashed,
    Heavy,
    LightLight,
    LightHeavy,
    HeavyLight,
    HeavyHeavy,
    Tick,
    Short,
    NoBarline,
}

/// A 1st/2nd (etc.) ending (volta) bracket. It is stored once, on the
/// [`Barline`] of the first measure it covers, and spans that measure plus
/// the following `measures - 1` measures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ending {
    /// Which pass(es) through the repeat this ending applies to, e.g. `[1]`
    /// or `[1, 2]`.
    pub numbers: Vec<u8>,
    /// How many measures the bracket covers, counting its first (at least 1).
    pub measures: u32,
    /// Whether the bracket's right end has a downward hook (usual for an
    /// ending that repeats back) rather than being left open (usual for the
    /// last ending).
    pub closed: bool,
}

/// The barline at the end of a measure: its visual style, repeat marks, and
/// any ending bracket.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Barline {
    pub kind: BarlineType,
    /// This measure begins a repeated section (a `|:` before it).
    pub repeat_start: bool,
    /// This measure ends a repeated section (a `:|` after it), with the
    /// number of times the section should be played (2 if unspecified).
    pub repeat_end: Option<u32>,
    /// An ending bracket beginning at this measure.
    pub ending: Option<Ending>,
}

/// A navigation/jump mark (D.C., D.S., Coda, Segno, Fine, ...) in a measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    Segno,
    Coda,
    DaCapo,
    DaCapoAlFine,
    DaCapoAlCoda,
    DalSegno,
    DalSegnoAlFine,
    DalSegnoAlCoda,
    Fine,
    ToCoda,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_c_is_midi_60() {
        let c4 = Pitch::new(Step::C, 0, 4);
        assert_eq!(c4.to_midi_number(), 60);
    }

    #[test]
    fn midi_round_trip_sharps() {
        for midi in 21..=108 {
            let pitch = Pitch::from_midi_number(midi, false);
            assert_eq!(pitch.to_midi_number(), midi);
        }
    }

    #[test]
    fn dotted_quarter_is_one_and_a_half_beats() {
        let d = Duration::dotted(NoteValue::Quarter, 1);
        assert_eq!(d.quarter_length(), 1.5);
    }

    #[test]
    fn triplet_eighth_length() {
        let d = Duration {
            base: NoteValue::Eighth,
            dots: 0,
            tuplet: Some(Tuplet {
                actual: 3,
                normal: 2,
            }),
        };
        assert!((d.quarter_length() - (1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn quantizes_to_nearest_plain_duration() {
        let d = Duration::from_quarter_length_quantized(1.5);
        assert_eq!(d.base, NoteValue::Quarter);
        assert_eq!(d.dots, 1);
    }

    #[test]
    fn single_voice_helper_wraps_events() {
        let measure = Measure::single_voice(
            1,
            MeasureAttributes::default(),
            vec![Event::Rest(Rest::new(Duration::default()))],
        );
        assert_eq!(measure.voices.len(), 1);
        assert_eq!(measure.voices[0].number, 1);
        assert_eq!(measure.voices[0].events.len(), 1);
    }
}
