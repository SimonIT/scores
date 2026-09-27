use guitarpro::model::legacy::chord::PitchClass;
use guitarpro::model::legacy::key_signature::Duration as GpDuration;
use guitarpro::{ChordAlteration, ChordExtension, ChordType, DirectionSign};
use scores_core::{
    ChordKind, ChordRoot, ChordSymbol, Duration, Dynamic, Navigation, NoteValue, Pitch, Tuplet,
};

/// Tuplet ratios (`enters`, `times`) Guitar Pro's format actually supports;
/// anything else is written as a plain (non-tuplet) duration instead.
pub(crate) const SUPPORTED_TUPLETS: [(u8, u8); 10] = [
    (1, 1),
    (3, 2),
    (5, 4),
    (6, 4),
    (7, 4),
    (9, 8),
    (10, 8),
    (11, 8),
    (12, 8),
    (13, 8),
];

pub(crate) fn gp_duration_to_model(d: &GpDuration) -> Duration {
    let base = match d.value {
        1 => NoteValue::Whole,
        2 => NoteValue::Half,
        4 => NoteValue::Quarter,
        8 => NoteValue::Eighth,
        16 => NoteValue::Sixteenth,
        32 => NoteValue::ThirtySecond,
        _ => NoteValue::SixtyFourth,
    };
    let dots = if d.double_dotted {
        2
    } else if d.dotted {
        1
    } else {
        0
    };
    let tuplet = (d.tuplet_enters > 1).then_some(Tuplet {
        actual: d.tuplet_enters,
        normal: d.tuplet_times,
    });
    Duration { base, dots, tuplet }
}

/// `guitarpro::Note::real_value` (fret + string tuning -> absolute MIDI
/// note) is crate-private, so it's reimplemented here - trivial, since the
/// track's tuning table (`strings: Vec<(string_number, midi_note)>`) is
/// itself public.
pub(crate) fn note_midi_value(note: &guitarpro::Note, strings: &[(i8, i8)]) -> Option<i32> {
    let index = usize::try_from(note.string).ok()?.checked_sub(1)?;
    let tuning = strings.get(index)?.1;
    Some(i32::from(note.value) + i32::from(tuning))
}

/// Guitar Pro's `repeat_alternative` is a bitmask where bit N means "part
/// of ending N+1" (a best-effort, commonly-used interpretation - the crate
/// doesn't document the exact semantics beyond the raw field type).
pub(crate) fn ending_numbers_from_bitmask(mask: u8) -> Vec<u8> {
    (0..8u8)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| bit + 1)
        .collect()
}

pub(crate) fn ending_bitmask_from_numbers(numbers: &[u8]) -> u8 {
    numbers
        .iter()
        .filter_map(|&n| n.checked_sub(1))
        .filter(|&bit| bit < 8)
        .fold(0u8, |mask, bit| mask | (1 << bit))
}

pub(crate) fn model_duration_to_gp(d: &Duration) -> GpDuration {
    let value = match d.base {
        NoteValue::Whole => 1,
        NoteValue::Half => 2,
        NoteValue::Quarter => 4,
        NoteValue::Eighth => 8,
        NoteValue::Sixteenth => 16,
        NoteValue::ThirtySecond => 32,
        NoteValue::SixtyFourth => 64,
    };
    let (tuplet_enters, tuplet_times) = match d.tuplet {
        Some(Tuplet { actual, normal }) if SUPPORTED_TUPLETS.contains(&(actual, normal)) => {
            (actual, normal)
        }
        _ => (1, 1),
    };
    GpDuration {
        value,
        dotted: d.dots >= 1,
        double_dotted: d.dots >= 2,
        tuplet_enters,
        tuplet_times,
        min_time: 0,
    }
}

/// Guitar Pro stores dynamics as one of 8 per-note velocity steps, ppp
/// (15) up to fff (127) in increments of 16 - the same values the crate's
/// (private) `unpack_velocity` produces. f (95) is the default, written
/// for every note that doesn't set a velocity of its own.
pub(crate) const DEFAULT_VELOCITY: i16 = 95;
const VELOCITY_STEPS: [(i16, Dynamic); 8] = [
    (15, Dynamic::Ppp),
    (31, Dynamic::Pp),
    (47, Dynamic::P),
    (63, Dynamic::Mp),
    (79, Dynamic::Mf),
    (95, Dynamic::F),
    (111, Dynamic::Ff),
    (127, Dynamic::Fff),
];

/// Snaps an arbitrary velocity onto the nearest of the 8 steps.
pub(crate) fn quantize_velocity(velocity: i16) -> i16 {
    let step = ((f32::from(velocity) + 1.0) / 16.0).round().clamp(1.0, 8.0) as i16;
    step * 16 - 1
}

pub(crate) fn velocity_to_dynamic(velocity: i16) -> Dynamic {
    let velocity = quantize_velocity(velocity);
    VELOCITY_STEPS
        .iter()
        .find(|(v, _)| *v == velocity)
        .map(|(_, d)| d.clone())
        .unwrap_or(Dynamic::F)
}

/// The velocity a note marked `dynamic` is played at, and the velocity
/// in effect for the notes after it (given `current`, the level before it).
/// Plain levels change both; momentary/compound markings (sfz, fp, ...)
/// only have 8 plain steps to land on, so they become a louder note that
/// then falls back to the previous (or, for fp-style ones, a softer) level.
pub(crate) fn dynamic_velocities(dynamic: &Dynamic, current: i16) -> (i16, i16) {
    let level = |v: i16| (v, v);
    match dynamic {
        Dynamic::Pppppp | Dynamic::Ppppp | Dynamic::Pppp | Dynamic::Ppp | Dynamic::N => level(15),
        Dynamic::Pp => level(31),
        Dynamic::P => level(47),
        Dynamic::Mp => level(63),
        Dynamic::Mf => level(79),
        Dynamic::F => level(95),
        Dynamic::Ff => level(111),
        Dynamic::Fff | Dynamic::Ffff | Dynamic::Fffff | Dynamic::Ffffff => level(127),
        Dynamic::Sf | Dynamic::Fz | Dynamic::Rf | Dynamic::Rfz => (111, current),
        Dynamic::Sfz | Dynamic::Sffz => (127, current),
        Dynamic::Fp => (95, 47),
        Dynamic::Sfp => (111, 47),
        Dynamic::Sfpp => (111, 31),
        Dynamic::Sfzp => (127, 47),
        Dynamic::Pf => (47, 95),
        Dynamic::Other(_) => level(current),
    }
}

/// A grace note's length as the crate stores it (`1 << (7 - byte)`, where
/// the file's byte is 1 = 32nd, 2 = 24th (triplet 16th), 3 = 16th - the
/// only three lengths Guitar Pro offers).
pub(crate) fn grace_duration_to_model(duration: u8) -> Duration {
    match duration {
        64.. => Duration::new(NoteValue::ThirtySecond),
        32 => Duration {
            base: NoteValue::Sixteenth,
            dots: 0,
            tuplet: Some(Tuplet {
                actual: 3,
                normal: 2,
            }),
        },
        _ => Duration::new(NoteValue::Sixteenth),
    }
}

/// Inverse of [`grace_duration_to_model`]: anything a 16th or longer
/// becomes a 16th, anything shorter a 32nd.
pub(crate) fn model_duration_to_grace(d: &Duration) -> u8 {
    if d.base == NoteValue::Sixteenth && d.tuplet.is_some() {
        32
    } else if d.quarter_length() >= NoteValue::Sixteenth.quarter_length() {
        16
    } else {
        64
    }
}

/// Tremolo picking speed (`TremoloPickingEffect::duration.value`: 8, 16 or
/// 32) <-> number of tremolo slashes on the stem.
pub(crate) fn tremolo_marks_from_speed(value: u16) -> u8 {
    match value {
        ..=8 => 1,
        9..=16 => 2,
        _ => 3,
    }
}

pub(crate) fn tremolo_speed_from_marks(marks: u8) -> u16 {
    match marks {
        ..=1 => 8,
        2 => 16,
        _ => 32,
    }
}

/// GP5's direction signs are a superset of ours: the "double coda" and
/// "segno segno" variants collapse onto their single counterparts, and
/// "Da Coda" is Guitar Pro's name for "To Coda".
pub(crate) fn direction_to_navigation(sign: &DirectionSign) -> Navigation {
    match sign {
        DirectionSign::Coda | DirectionSign::DoubleCoda => Navigation::Coda,
        DirectionSign::Segno | DirectionSign::SegnoSegno => Navigation::Segno,
        DirectionSign::Fine => Navigation::Fine,
        DirectionSign::DaCapo => Navigation::DaCapo,
        DirectionSign::DaCapoAlCoda | DirectionSign::DaCapoAlDoubleCoda => Navigation::DaCapoAlCoda,
        DirectionSign::DaCapoAlFine => Navigation::DaCapoAlFine,
        DirectionSign::DaSegno | DirectionSign::DaSegnoSegno => Navigation::DalSegno,
        DirectionSign::DaSegnoAlCoda
        | DirectionSign::DaSegnoAlDoubleCoda
        | DirectionSign::DaSegnoSegnoAlCoda
        | DirectionSign::DaSegnoSegnoAlDoubleCoda => Navigation::DalSegnoAlCoda,
        DirectionSign::DaSegnoAlFine | DirectionSign::DaSegnoSegnoAlFine => {
            Navigation::DalSegnoAlFine
        }
        DirectionSign::DaCoda | DirectionSign::DaDoubleCoda => Navigation::ToCoda,
    }
}

pub(crate) fn navigation_to_direction(navigation: Navigation) -> DirectionSign {
    match navigation {
        Navigation::Segno => DirectionSign::Segno,
        Navigation::Coda => DirectionSign::Coda,
        Navigation::DaCapo => DirectionSign::DaCapo,
        Navigation::DaCapoAlFine => DirectionSign::DaCapoAlFine,
        Navigation::DaCapoAlCoda => DirectionSign::DaCapoAlCoda,
        Navigation::DalSegno => DirectionSign::DaSegno,
        Navigation::DalSegnoAlFine => DirectionSign::DaSegnoAlFine,
        Navigation::DalSegnoAlCoda => DirectionSign::DaSegnoAlCoda,
        Navigation::Fine => DirectionSign::Fine,
        Navigation::ToCoda => DirectionSign::DaCoda,
    }
}

/// Maximum length of a chord's name in the GP4/GP5 chord block.
const CHORD_NAME_MAX: usize = 22;

/// Reads a beat's chord diagram as a chord symbol. The free-text `name`
/// (what Guitar Pro displays) wins when it parses; the structured
/// root/type fields (GP4+ "new format" chords only) are the fallback for
/// unnamed or unparseable ones.
pub(crate) fn chord_to_symbol(chord: &guitarpro::Chord) -> Option<ChordSymbol> {
    ChordSymbol::parse(&chord.name).or_else(|| {
        let root = pitch_class_to_root(chord.root.as_ref()?);
        let bass = chord
            .bass
            .as_ref()
            .map(pitch_class_to_root)
            .filter(|bass| bass.pitch_class() != root.pitch_class());
        Some(ChordSymbol {
            root,
            kind: chord_kind_from_gp(chord),
            bass,
        })
    })
}

fn pitch_class_to_root(pc: &PitchClass) -> ChordRoot {
    let pitch = Pitch::from_midi_number(i32::from(pc.value), !pc.sharp);
    ChordRoot::new(pitch.step, pitch.alter)
}

fn chord_kind_from_gp(chord: &guitarpro::Chord) -> ChordKind {
    let extension = chord.extension.clone().unwrap_or(ChordExtension::None);
    let fifth = chord.fifth.clone().unwrap_or(ChordAlteration::Perfect);
    let Some(kind) = &chord.kind else {
        return ChordKind::Major;
    };
    match (kind, extension, fifth) {
        (ChordType::Seventh, ChordExtension::Ninth, _) => ChordKind::DominantNinth,
        (ChordType::Seventh, ChordExtension::Eleventh, _) => ChordKind::Dominant11th,
        (ChordType::Seventh, ChordExtension::Thirteenth, _) => ChordKind::Dominant13th,
        (ChordType::Seventh, _, ChordAlteration::Augmented) => ChordKind::AugmentedSeventh,
        (ChordType::MajorSeventh, ChordExtension::Ninth, _) => ChordKind::MajorNinth,
        (ChordType::MajorSeventh, ChordExtension::Eleventh, _) => ChordKind::Major11th,
        (ChordType::MajorSeventh, ChordExtension::Thirteenth, _) => ChordKind::Major13th,
        (ChordType::MinorSeventh, ChordExtension::Ninth, _) => ChordKind::MinorNinth,
        (ChordType::MinorSeventh, ChordExtension::Eleventh, _) => ChordKind::Minor11th,
        (ChordType::MinorSeventh, ChordExtension::Thirteenth, _) => ChordKind::Minor13th,
        (ChordType::MinorSeventh, _, ChordAlteration::Diminished) => ChordKind::HalfDiminished,
        (ChordType::Major, ..) => ChordKind::Major,
        (ChordType::Seventh, ..) => ChordKind::Dominant,
        (ChordType::MajorSeventh, ..) => ChordKind::MajorSeventh,
        (ChordType::Sixth, ..) => ChordKind::MajorSixth,
        (ChordType::Minor, ..) => ChordKind::Minor,
        (ChordType::MinorSeventh, ..) => ChordKind::MinorSeventh,
        (ChordType::MinorMajor, ..) => ChordKind::MajorMinor,
        (ChordType::MinorSixth, ..) => ChordKind::MinorSixth,
        (ChordType::SuspendedSecond, ..) => ChordKind::SuspendedSecond,
        (ChordType::SuspendedFourth, ..) => ChordKind::SuspendedFourth,
        (ChordType::SeventhSuspendedSecond, ..) => ChordKind::Other("7sus2".into()),
        (ChordType::SeventhSuspendedFourth, ..) => ChordKind::Other("7sus4".into()),
        (ChordType::Diminished, ..) => ChordKind::Diminished,
        (ChordType::Augmented, ..) => ChordKind::Augmented,
        (ChordType::Power, ..) => ChordKind::Power,
        (ChordType::Unknown(_), ..) => ChordKind::Major,
    }
}

/// Builds a name-only chord (no diagram: every string's fret is left at
/// the writer's "unplayed" default) carrying the symbol both as display
/// text and, as far as Guitar Pro's chord vocabulary reaches, in the
/// structured root/type/extension/bass fields.
pub(crate) fn symbol_to_chord(symbol: &ChordSymbol) -> guitarpro::Chord {
    // The writer pads the name to 22 bytes after Windows-1252 encoding it,
    // assuming one byte per char - keep it ASCII (our text form always is,
    // barring an exotic `ChordKind::Other` suffix) and short enough.
    let name: String = symbol
        .to_string()
        .chars()
        .filter(char::is_ascii)
        .take(CHORD_NAME_MAX)
        .collect();
    let (kind, extension, fifth) = chord_kind_to_gp(&symbol.kind);
    let sharp = symbol.root.alter >= 0;
    guitarpro::Chord {
        length: 7,
        new_format: Some(true),
        sharp: Some(sharp),
        root: Some(root_to_pitch_class(symbol.root)),
        kind: Some(kind),
        extension: Some(extension),
        // Guitar Pro stores the bass explicitly even for non-slash chords.
        bass: Some(root_to_pitch_class(symbol.bass.unwrap_or(symbol.root))),
        tonality: Some(ChordAlteration::Perfect),
        add: Some(false),
        name,
        fifth: Some(fifth),
        ninth: Some(ChordAlteration::Perfect),
        eleventh: Some(ChordAlteration::Perfect),
        first_fret: Some(1),
        show: Some(true),
        ..guitarpro::Chord::default()
    }
}

fn root_to_pitch_class(root: ChordRoot) -> PitchClass {
    let value = root.pitch_class() as i8;
    let accidental = root.alter.signum();
    PitchClass {
        note: root.to_string(),
        just: (value - accidental).rem_euclid(12),
        accidental,
        value,
        sharp: root.alter >= 0,
    }
}

fn chord_kind_to_gp(kind: &ChordKind) -> (ChordType, ChordExtension, ChordAlteration) {
    use ChordAlteration as A;
    use ChordExtension as E;
    match kind {
        ChordKind::Major | ChordKind::NoChord | ChordKind::Other(_) => {
            (ChordType::Major, E::None, A::Perfect)
        }
        ChordKind::Minor => (ChordType::Minor, E::None, A::Perfect),
        ChordKind::Augmented => (ChordType::Augmented, E::None, A::Perfect),
        ChordKind::Diminished | ChordKind::DiminishedSeventh => {
            (ChordType::Diminished, E::None, A::Perfect)
        }
        ChordKind::Dominant => (ChordType::Seventh, E::None, A::Perfect),
        ChordKind::MajorSeventh => (ChordType::MajorSeventh, E::None, A::Perfect),
        ChordKind::MinorSeventh => (ChordType::MinorSeventh, E::None, A::Perfect),
        ChordKind::AugmentedSeventh => (ChordType::Seventh, E::None, A::Augmented),
        ChordKind::HalfDiminished => (ChordType::MinorSeventh, E::None, A::Diminished),
        ChordKind::MajorMinor => (ChordType::MinorMajor, E::None, A::Perfect),
        ChordKind::MajorSixth => (ChordType::Sixth, E::None, A::Perfect),
        ChordKind::MinorSixth => (ChordType::MinorSixth, E::None, A::Perfect),
        ChordKind::DominantNinth => (ChordType::Seventh, E::Ninth, A::Perfect),
        ChordKind::MajorNinth => (ChordType::MajorSeventh, E::Ninth, A::Perfect),
        ChordKind::MinorNinth => (ChordType::MinorSeventh, E::Ninth, A::Perfect),
        ChordKind::Dominant11th => (ChordType::Seventh, E::Eleventh, A::Perfect),
        ChordKind::Major11th => (ChordType::MajorSeventh, E::Eleventh, A::Perfect),
        ChordKind::Minor11th => (ChordType::MinorSeventh, E::Eleventh, A::Perfect),
        ChordKind::Dominant13th => (ChordType::Seventh, E::Thirteenth, A::Perfect),
        ChordKind::Major13th => (ChordType::MajorSeventh, E::Thirteenth, A::Perfect),
        ChordKind::Minor13th => (ChordType::MinorSeventh, E::Thirteenth, A::Perfect),
        ChordKind::SuspendedSecond => (ChordType::SuspendedSecond, E::None, A::Perfect),
        ChordKind::SuspendedFourth => (ChordType::SuspendedFourth, E::None, A::Perfect),
        ChordKind::Power => (ChordType::Power, E::None, A::Perfect),
    }
}
