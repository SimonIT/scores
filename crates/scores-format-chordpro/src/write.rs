use crate::convert::{
    FERMATA_TOKEN, dynamic_to_token, navigation_precedes_measure, navigation_to_text,
};
use scores_core::{
    ChordKind, ChordRoot, ChordSymbol, Dynamic, Event, Measure, Part, Pitch, Result, Score,
};

/// Chord qualities [`recognize_chord`] tries to match a note set against.
/// Matching is by exact pitch-class set, so order doesn't matter.
const RECOGNIZED_KINDS: &[ChordKind] = &[
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
];

/// ChordPro is a chords+lyrics lead-sheet format with no multi-part concept
/// and no notated rhythm, so only one part is written, one chord line per
/// measure, in one of two modes:
///
/// - **Chord-symbol mode**, if any part carries a [`ChordSymbol`] anywhere:
///   the first such part is written as its chord symbols only (from all
///   voices, in onset order), since in a lead sheet the notes under the
///   symbols are the melody, not the harmony. Events without a symbol are
///   skipped, as are measures with no symbol at all (a chord simply holds
///   until the next one, which is all a ChordPro reader can assume anyway).
/// - **Pitch-set mode** otherwise: the first part's first voice, with every
///   event reduced to a chord symbol via [`recognize_chord`] - a
///   best-effort reverse of what the reader does, not a full harmonic
///   analyzer. Rests become a blank chord position.
///
/// There is no real lyric text to write (our model doesn't carry any), so a
/// single space is written after each chord to keep the output valid,
/// readable ChordPro.
///
/// Dynamics/fermatas of a written event (from the first note of a chord,
/// the note itself, or a rest's fermata) are written as this crate's
/// bolt-on `<token>` marker convention - see [`crate::convert`]. A
/// measure's navigation marks are each written as a standard
/// `{comment: ...}` line before (segno, coda) or after (D.C., D.S., Fine,
/// To Coda) its chord line.
pub fn write(score: &Score) -> Result<Vec<u8>> {
    let mut out = String::new();
    if let Some(title) = &score.title {
        out.push_str(&format!("{{title: {title}}}\n"));
    }
    if let Some(composer) = &score.composer {
        out.push_str(&format!("{{artist: {composer}}}\n"));
    }
    out.push('\n');

    let symbol_part = score.parts.iter().find(|part| has_chord_symbols(part));
    if let Some(part) = symbol_part.or(score.parts.first()) {
        for measure in &part.measures {
            let navigation = &measure.navigation;
            for &mark in navigation
                .iter()
                .filter(|&&n| navigation_precedes_measure(n))
            {
                out.push_str(&format!("{{comment: {}}}\n", navigation_to_text(mark)));
            }

            let line = if symbol_part.is_some() {
                symbol_line(measure)
            } else {
                pitch_set_line(measure)
            };
            if !line.is_empty() {
                out.push_str(&line);
                out.push('\n');
            }

            for &mark in navigation
                .iter()
                .filter(|&&n| !navigation_precedes_measure(n))
            {
                out.push_str(&format!("{{comment: {}}}\n", navigation_to_text(mark)));
            }
        }
    }

    Ok(out.into_bytes())
}

fn has_chord_symbols(part: &Part) -> bool {
    part.measures
        .iter()
        .flat_map(|m| &m.voices)
        .flat_map(|v| &v.events)
        .any(|e| e.chord_symbol().is_some())
}

/// A measure's chord symbols from all its voices, merged by onset (a stable
/// sort, so simultaneous symbols keep voice order; an identical symbol at
/// the same onset in another voice is written once).
fn symbol_line(measure: &Measure) -> String {
    let mut symbols: Vec<(f64, &Event, &ChordSymbol)> = Vec::new();
    for voice in &measure.voices {
        let mut onset = 0.0;
        for event in &voice.events {
            if let Some(symbol) = event.chord_symbol() {
                symbols.push((onset, event, symbol));
            }
            if !is_grace(event) {
                onset += event.duration().quarter_length();
            }
        }
    }
    symbols.sort_by(|a, b| a.0.total_cmp(&b.0));
    symbols.dedup_by(|b, a| a.0 == b.0 && a.2 == b.2);

    let mut line = String::new();
    for (_, event, symbol) in symbols {
        push_markers(&mut line, event);
        line.push_str(&format!("[{}] ", chord_text(symbol)));
    }
    line
}

fn pitch_set_line(measure: &Measure) -> String {
    let mut line = String::new();
    let Some(voice) = measure.voices.first() else {
        return line;
    };
    for event in &voice.events {
        let pitches: Vec<Pitch> = match event {
            Event::Note(note) => vec![note.pitch],
            Event::Chord(notes) => notes.iter().map(|n| n.pitch).collect(),
            Event::Rest(_) => Vec::new(),
        };
        push_markers(&mut line, event);
        match recognize_chord(&pitches) {
            Some(symbol) => line.push_str(&format!("[{}] ", chord_text(&symbol))),
            None => line.push_str("  "),
        }
    }
    line
}

fn is_grace(event: &Event) -> bool {
    match event {
        Event::Note(note) => note.grace,
        Event::Chord(notes) => notes.first().is_some_and(|n| n.grace),
        Event::Rest(_) => false,
    }
}

fn push_markers(line: &mut String, event: &Event) {
    let (dynamic, fermata): (Option<&Dynamic>, bool) = match event {
        Event::Note(note) => (note.dynamic.as_ref(), note.fermata),
        Event::Chord(notes) => notes
            .first()
            .map_or((None, false), |n| (n.dynamic.as_ref(), n.fermata)),
        Event::Rest(rest) => (None, rest.fermata),
    };
    if let Some(token) = dynamic.and_then(dynamic_to_token) {
        line.push_str(&format!("<{token}>"));
    }
    if fermata {
        line.push_str(&format!("<{FERMATA_TOKEN}>"));
    }
}

/// A chord symbol's text form, minus any characters that would break out
/// of the `[...]` it's written in (only possible in a verbatim
/// [`ChordKind::Other`] suffix from another format).
fn chord_text(symbol: &ChordSymbol) -> String {
    symbol
        .to_string()
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | '{' | '}' | '\n' | '\r'))
        .collect()
}

/// Matches the note set's pitch-class set exactly against each
/// [`RECOGNIZED_KINDS`] entry, trying the lowest note as root first (so a
/// root-position chord is never misnamed as an inversion of another, e.g.
/// A-C-E-G is `Am7`, not `C6/A`) and then every other pitch class present
/// (giving a slash chord). Falls back to a bare major triad on the lowest
/// note if nothing matches; `None` for an empty set (a rest). The root and
/// bass keep the spelling of the notes they came from.
fn recognize_chord(pitches: &[Pitch]) -> Option<ChordSymbol> {
    let lowest = pitches.iter().min_by_key(|p| p.to_midi_number())?;
    let bass_class = pitch_class(lowest);
    let root_of = |class: u8| {
        let pitch = pitches
            .iter()
            .find(|p| pitch_class(p) == class)
            .expect("class taken from pitches");
        ChordRoot::new(pitch.step, pitch.alter)
    };

    let mut classes: Vec<u8> = pitches.iter().map(pitch_class).collect();
    classes.sort_unstable();
    classes.dedup();
    // Lowest note's class first; the rest keep ascending order.
    let mut roots = classes.clone();
    roots.sort_by_key(|&c| c != bass_class);

    for &root_class in &roots {
        for kind in RECOGNIZED_KINDS {
            let mut kind_classes: Vec<u8> = kind
                .intervals()
                .iter()
                .map(|i| (root_class + i) % 12)
                .collect();
            kind_classes.sort_unstable();
            kind_classes.dedup();
            if kind_classes == classes {
                let mut symbol = ChordSymbol::new(root_of(root_class), kind.clone());
                if root_class != bass_class {
                    symbol.bass = Some(root_of(bass_class));
                }
                return Some(symbol);
            }
        }
    }

    Some(ChordSymbol::new(root_of(bass_class), ChordKind::Major))
}

fn pitch_class(pitch: &Pitch) -> u8 {
    pitch.to_midi_number().rem_euclid(12) as u8
}
