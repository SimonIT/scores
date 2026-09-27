//! Converts an [`abc_parser`] parse tree into a [`Score`].
//!
//! `abc_parser` handles the actual ABC grammar (accidentals, octave marks,
//! ties, chords, tuplets, broken rhythm, decorations, comments, ...); this
//! module only maps its AST onto our format-agnostic model. `V:n` inline
//! fields switch which [`scores_core::Voice`] subsequent symbols go into.
//!
//! Decorations (both the single-letter shorthands and ABC 2.1 `!name!`
//! forms, see [`marking_from_name`]) map onto articulations, ornaments,
//! fermatas, dynamics (`!p!`, `!mf!`, `!sfz!`, ...) and hairpins
//! (`!<(!`/`!crescendo(!` ... `!<)!`), and apply to the next note, chord or
//! rest (a rest only keeps a fermata; its other marks are dropped). In a
//! chord, dynamics/hairpins/chord symbols go on the first note only while
//! articulations/ornaments/fermatas go on every note; decorations written
//! *inside* the brackets (`[.C!tenuto!E]`) apply to their own note.
//! Unrecognized `!name!` decorations become `Articulation::Other(name)`.
//! Navigation decorations (`S`, `O`, `!D.C.!`, `!fine!`, `!dacoda!`, ...)
//! are added to the enclosing measure's [`Navigation`] marks (in written
//! order) instead of being per-note. A quoted annotation without a placement prefix (`"Am7"`) is an ABC
//! chord symbol and is parsed with [`ChordSymbol::parse`]; placed
//! annotations (`"^text"`) and unparseable ones are ignored.
//!
//! Bar tokens carry the barline style (`|`, `||`, `|]`, `[|`, `.|`, `[|]`),
//! repeat marks and endings, split the way our model wants them: the style
//! and a `:|` belong to the measure the token closes, while a `|:` and an
//! ending number (`|1`, `:|2`, `[1,2`, `|1-3`) belong to the measure that
//! follows it. A bar token with nothing before it on its line (a tune
//! starting with `|:`) just opens the next measure rather than closing an
//! empty one. ABC has no end-of-ending token, so each ending's extent is
//! inferred (see [`infer_ending_extents`]).
//!
//! A `{...}` grace-note group becomes one grace [`Note`] per inner note
//! (the acciaccatura `/` marker isn't distinguished - our model has no
//! equivalent); they don't consume a tuplet slot or absorb a pending
//! decoration (both apply to the following main note instead, matching how
//! they read musically). Limitations: mid-tune field changes other than
//! `V:` (`[K:...]`, `[M:...]`, ...) are ignored.

use abc_parser::abc;
use abc_parser::datatypes::writer::ToABC;
use abc_parser::datatypes::{
    Accidental, Annotation, Decoration, HeaderLine, InfoField, Length as AbcLength, MusicLine,
    MusicSymbol, Note as AbcNote, Rest as AbcRest, Tie as AbcTie, Tune, TuneBody, TuneLine,
};
use scores_core::{
    Articulation, Barline, BarlineType, ChordSymbol, Clef, Duration, Dynamic, Ending, Event,
    KeySignature, Measure, MeasureAttributes, Mode, Navigation, Note, Ornament, Part, Pitch, Rest,
    Result, Score, ScoresError, Step, Tempo, Tie, TimeSignature, Tuplet, Voice, Wedge,
};
use std::collections::BTreeMap;

pub fn read(data: &[u8]) -> Result<Score> {
    let text = std::str::from_utf8(data)
        .map_err(|e| ScoresError::Parse(format!("input is not valid utf-8: {e}")))?;

    let tune_book = abc::tune_book(text)
        .map_err(|e| ScoresError::Parse(format!("invalid ABC notation: {e}")))?;

    if tune_book.tunes.is_empty() {
        return Err(ScoresError::Parse(
            "no ABC tune found (expected an X: header)".into(),
        ));
    }

    let mut score = Score::default();
    for (index, tune) in tune_book.tunes.iter().enumerate() {
        let (part, title, composer) = read_tune(tune, index + 1)?;
        if index == 0 {
            score.title = title;
            score.composer = composer;
        }
        score.parts.push(part);
    }

    Ok(score)
}

fn read_tune(tune: &Tune, index: usize) -> Result<(Part, Option<String>, Option<String>)> {
    let mut title: Option<String> = None;
    let mut composer: Option<String> = None;
    let mut time_signature = TimeSignature::default();
    let mut key_signature = KeySignature::default();
    let mut tempo: Option<Tempo> = None;
    let mut unit_length: Option<f64> = None;
    let mut part_id = format!("P{index}");

    for line in &tune.header.lines {
        if let HeaderLine::Field(InfoField(field, value), _) = line {
            match field {
                'X' => part_id = format!("P{}", value.trim()),
                'T' if title.is_none() => title = Some(value.trim().to_string()),
                'C' if composer.is_none() => composer = Some(value.trim().to_string()),
                'M' => time_signature = parse_meter(value.trim()),
                'L' => unit_length = Some(parse_unit_length(value.trim())),
                'Q' => tempo = Some(parse_tempo(value.trim())),
                'K' => key_signature = parse_key(value.trim()),
                _ => {}
            }
        }
    }

    let unit_length = unit_length.unwrap_or_else(|| default_unit_length(&time_signature));
    let measure_quarter_length =
        time_signature.beats as f64 * 4.0 / time_signature.beat_type as f64;

    let mut state = MultiVoiceState::new(unit_length, measure_quarter_length);
    if let Some(body) = &tune.body {
        state.process_body(body);
    }
    let mut measures = state.finish();
    infer_ending_extents(&mut measures);

    if measures.is_empty() {
        measures.push(Measure::default());
    }
    let first = measures.first_mut().expect("just ensured non-empty");
    first.attributes = MeasureAttributes {
        time_signature: Some(time_signature),
        key_signature: Some(key_signature),
        clef: Some(Clef::Treble),
        tempo,
    };

    Ok((
        Part {
            id: part_id,
            name: title.clone(),
            measures,
        },
        title,
        composer,
    ))
}

fn parse_meter(value: &str) -> TimeSignature {
    if value.eq_ignore_ascii_case("C") {
        return TimeSignature {
            beats: 4,
            beat_type: 4,
        };
    }
    if value == "C|" {
        return TimeSignature {
            beats: 2,
            beat_type: 2,
        };
    }
    if let Some((num, den)) = value.split_once('/')
        && let (Ok(beats), Ok(beat_type)) = (num.trim().parse(), den.trim().parse())
    {
        return TimeSignature { beats, beat_type };
    }
    TimeSignature::default()
}

/// Converts an `L:` field like `1/8` into a length in quarter notes.
fn parse_unit_length(value: &str) -> f64 {
    if let Some((num, den)) = value.split_once('/') {
        let n: f64 = num.trim().parse().unwrap_or(1.0);
        let d: f64 = den.trim().parse().unwrap_or(8.0);
        return 4.0 * n / d;
    }
    0.5
}

/// Per the ABC standard, the default unit note length is 1/8 unless the
/// meter's ratio is below 3/4, in which case it is 1/16.
fn default_unit_length(time_signature: &TimeSignature) -> f64 {
    let ratio = time_signature.beats as f64 / time_signature.beat_type as f64;
    if ratio < 0.75 { 4.0 / 16.0 } else { 4.0 / 8.0 }
}

fn parse_tempo(value: &str) -> Tempo {
    let bpm_part = value.rsplit('=').next().unwrap_or(value);
    match bpm_part.trim().parse() {
        Ok(bpm) => Tempo { bpm },
        Err(_) => Tempo::default(),
    }
}

fn parse_key(value: &str) -> KeySignature {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return KeySignature::default();
    }

    let mut chars = value.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return KeySignature::default(),
    };
    let mut tonic = first.to_ascii_uppercase().to_string();
    let mut rest = chars.as_str();
    if let Some(c) = rest.chars().next()
        && (c == '#' || c == 'b')
    {
        tonic.push(c);
        rest = &rest[1..];
    }

    let rest_trim = rest.trim_start();
    let lower = rest_trim.to_ascii_lowercase();
    let is_minor = lower.starts_with('m') && !lower.starts_with("maj");

    if is_minor {
        KeySignature {
            fifths: crate::tonic_to_fifths_minor(&tonic),
            mode: Mode::Minor,
        }
    } else {
        KeySignature {
            fifths: crate::tonic_to_fifths(&tonic),
            mode: Mode::Major,
        }
    }
}

/// Reads the `f32` a parsed [`AbcLength`] wraps via its `Debug` output
/// (`"Length(1.5)"`) since the crate does not expose it directly.
fn length_value(length: &AbcLength) -> f64 {
    let debug = format!("{length:?}");
    debug
        .trim_start_matches("Length(")
        .trim_end_matches(')')
        .parse()
        .unwrap_or(1.0)
}

fn to_step(note: AbcNote) -> Step {
    match note {
        AbcNote::A => Step::A,
        AbcNote::B => Step::B,
        AbcNote::C => Step::C,
        AbcNote::D => Step::D,
        AbcNote::E => Step::E,
        AbcNote::F => Step::F,
        AbcNote::G => Step::G,
    }
}

fn alter_from_accidental(accidental: Accidental) -> i8 {
    match accidental {
        Accidental::Natural => 0,
        Accidental::Sharp => 1,
        Accidental::Flat => -1,
        Accidental::DoubleSharp => 2,
        Accidental::DoubleFlat => -2,
    }
}

/// The tuplet ratio in effect for a run of upcoming notes/rests/chords.
struct TupletContext {
    remaining: u32,
    tuplet: Tuplet,
}

/// ABC gives no default "normal" count for a tuplet's `(p` shorthand; this
/// follows the common convention (not meter-sensitive, unlike the full
/// spec) used by most tunes in 4/4-like time.
fn default_tuplet_normal(p: u32) -> u32 {
    match p {
        2 => 3,
        3 => 2,
        4 => 3,
        6 => 2,
        _ => 3,
    }
}

/// A raw ABC bar token (`"|"`, `"|:"`, `":|"`, `"::"`, `"|]"`, `".|"`, ...)
/// plus any ending number written straight after it, split into the part
/// that closes the measure before it (`kind`, `repeat_end`) and the part
/// that opens the measure after it (`repeat_start`, `ending`). ABC (like
/// our model) has only one barline per boundary, not a left/right pair.
struct BarToken {
    kind: BarlineType,
    repeat_end: Option<u32>,
    repeat_start: bool,
    ending: Option<Ending>,
}

fn parse_bar_token(text: &str, ending: Option<&str>) -> BarToken {
    let (dotted, body) = match text.strip_prefix('.') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let kind = match body.trim_matches(':') {
        "" | "|" if dotted => BarlineType::Dotted,
        "" | "|" => BarlineType::Regular,
        "||" => BarlineType::LightLight,
        "|]" => BarlineType::LightHeavy,
        "[|" => BarlineType::HeavyLight,
        "[|]" | "[]" => BarlineType::NoBarline,
        "[" | "]" => BarlineType::Heavy,
        "][" | "]|[" => BarlineType::HeavyHeavy,
        other if other.contains(']') => BarlineType::LightHeavy,
        other if other.matches('|').count() >= 2 => BarlineType::LightLight,
        _ => BarlineType::Regular,
    };
    BarToken {
        kind,
        repeat_end: body.starts_with(':').then_some(2),
        repeat_start: body.ends_with(':'),
        ending: ending.map(ending_start),
    }
}

/// An ending starting here; its extent is filled in afterwards by
/// [`infer_ending_extents`].
fn ending_start(spec: &str) -> Ending {
    Ending {
        numbers: parse_ending_numbers(spec),
        measures: 1,
        closed: false,
    }
}

/// ABC only marks where an ending begins, so each ending's extent is
/// inferred: it runs from its first measure up to and including the first
/// measure that ends with a `:|` (a closed bracket), or failing that, the
/// first one ending with a non-plain barline (`||`, `|]`, ...; an open
/// bracket). The search stops before the next ending or `|:`; if neither
/// kind of measure is found by then, the ending is an open one-measure
/// bracket.
fn infer_ending_extents(measures: &mut [Measure]) {
    for start in 0..measures.len() {
        if measures[start].barline.ending.is_none() {
            continue;
        }
        let mut extent = (1, false);
        for (offset, measure) in measures[start..].iter().enumerate() {
            let barline = &measure.barline;
            if offset > 0 && (barline.ending.is_some() || barline.repeat_start) {
                break;
            }
            if barline.repeat_end.is_some() {
                extent = (offset + 1, true);
                break;
            }
            if barline.kind != BarlineType::Regular {
                extent = (offset + 1, false);
                break;
            }
        }
        let ending = measures[start]
            .barline
            .ending
            .as_mut()
            .expect("checked above");
        ending.measures = extent.0 as u32;
        ending.closed = extent.1;
    }
}

/// Parses a comma-separated ending spec like `"1"`, `"1,2"` or `"1-3"`
/// (a range is expanded into every number it spans).
fn parse_ending_numbers(spec: &str) -> Vec<u8> {
    let mut numbers = Vec::new();
    for part in spec.split(',') {
        let mut bounds = part.split('-').map(|n| n.trim().parse::<u8>());
        match (bounds.next(), bounds.next()) {
            (Some(Ok(start)), Some(Ok(end))) if start <= end => numbers.extend(start..=end),
            (Some(Ok(start)), _) => numbers.push(start),
            _ => {}
        }
    }
    numbers
}

/// What a decoration means in our model.
enum Marking {
    Articulation(Articulation),
    Ornament(Ornament),
    Fermata,
    Dynamic(Dynamic),
    Wedge(Wedge),
    Navigation(Navigation),
    /// Recognized, but carries nothing we model (e.g. a wavy line's end).
    Ignored,
}

fn marking_from_decoration(decoration: &Decoration) -> Marking {
    match decoration {
        Decoration::Staccato => Marking::Articulation(Articulation::Staccato),
        Decoration::Accent => Marking::Articulation(Articulation::Accent),
        Decoration::UpBow => Marking::Articulation(Articulation::Other("up-bow".to_string())),
        Decoration::DownBow => Marking::Articulation(Articulation::Other("down-bow".to_string())),
        Decoration::Trill => Marking::Ornament(Ornament::TrillMark),
        Decoration::LowerMordent => Marking::Ornament(Ornament::Mordent),
        Decoration::UpperMordent => Marking::Ornament(Ornament::InvertedMordent),
        Decoration::Roll => Marking::Ornament(Ornament::Other("roll".to_string())),
        Decoration::Fermata => Marking::Fermata,
        Decoration::Coda => Marking::Navigation(Navigation::Coda),
        Decoration::Segno => Marking::Navigation(Navigation::Segno),
        Decoration::Unresolved(name) => marking_from_name(name),
    }
}

/// Maps an ABC 2.1 `!name!` decoration (plus the common abcm2ps extensions
/// `!marcato!`, `!D.C.alfine!`-style jumps and `!/!`..`!////!` tremolos).
fn marking_from_name(name: &str) -> Marking {
    if let Some(dynamic) = dynamic_from_name(name) {
        return Marking::Dynamic(dynamic);
    }
    match name {
        "staccato" => Marking::Articulation(Articulation::Staccato),
        ">" | "accent" | "emphasis" => Marking::Articulation(Articulation::Accent),
        "marcato" => Marking::Articulation(Articulation::StrongAccent),
        "tenuto" => Marking::Articulation(Articulation::Tenuto),
        "wedge" => Marking::Articulation(Articulation::Staccatissimo),
        "breath" => Marking::Articulation(Articulation::BreathMark),
        "slide" => Marking::Articulation(Articulation::Scoop),
        "upbow" => Marking::Articulation(Articulation::Other("up-bow".to_string())),
        "downbow" => Marking::Articulation(Articulation::Other("down-bow".to_string())),
        "trill" => Marking::Ornament(Ornament::TrillMark),
        "trill(" => Marking::Ornament(Ornament::WavyLine),
        "trill)" => Marking::Ignored,
        "lowermordent" | "mordent" => Marking::Ornament(Ornament::Mordent),
        "uppermordent" | "pralltriller" => Marking::Ornament(Ornament::InvertedMordent),
        "roll" => Marking::Ornament(Ornament::Other("roll".to_string())),
        "turn" => Marking::Ornament(Ornament::Turn),
        "invertedturn" => Marking::Ornament(Ornament::InvertedTurn),
        "/" | "//" | "///" | "////" => Marking::Ornament(Ornament::Tremolo(name.len() as u8)),
        "fermata" | "invertedfermata" => Marking::Fermata,
        "crescendo(" | "<(" => Marking::Wedge(Wedge::CrescendoStart),
        "diminuendo(" | ">(" => Marking::Wedge(Wedge::DiminuendoStart),
        "crescendo)" | "<)" | "diminuendo)" | ">)" => Marking::Wedge(Wedge::Stop),
        "segno" => Marking::Navigation(Navigation::Segno),
        "coda" => Marking::Navigation(Navigation::Coda),
        "D.C." | "dacapo" => Marking::Navigation(Navigation::DaCapo),
        "D.C.alfine" => Marking::Navigation(Navigation::DaCapoAlFine),
        "D.C.alcoda" => Marking::Navigation(Navigation::DaCapoAlCoda),
        "D.S." => Marking::Navigation(Navigation::DalSegno),
        "D.S.alfine" => Marking::Navigation(Navigation::DalSegnoAlFine),
        "D.S.alcoda" => Marking::Navigation(Navigation::DalSegnoAlCoda),
        "fine" => Marking::Navigation(Navigation::Fine),
        "dacoda" => Marking::Navigation(Navigation::ToCoda),
        other => Marking::Articulation(Articulation::Other(other.to_string())),
    }
}

/// The dynamic a decoration name spells (`"mf"` -> [`Dynamic::Mf`]), for
/// every named variant of our model - [`crate::dynamic_name`] in reverse.
fn dynamic_from_name(name: &str) -> Option<Dynamic> {
    crate::DYNAMICS
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(d, _)| d.clone())
}

/// The chord symbol an ABC annotation spells, if it is one: only an
/// annotation without a placement prefix (`"Am7"`, not `"^Am7"`) is a chord
/// symbol in ABC, and [`ChordSymbol::parse`] rejects the prefixed ones
/// anyway since they don't start with a note name. `abc_parser` keeps the
/// annotation's text private, so it's recovered from its ABC form.
fn chord_symbol_from_annotation(annotation: &Annotation) -> Option<ChordSymbol> {
    let text = annotation.to_abc();
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    ChordSymbol::parse(inner)
}

/// Decorations/chord symbols seen but not yet attached to a note, chord or
/// rest.
#[derive(Default)]
struct PendingMarks {
    articulations: Vec<Articulation>,
    ornaments: Vec<Ornament>,
    fermata: bool,
    dynamic: Option<Dynamic>,
    wedge: Option<Wedge>,
    chord_symbol: Option<ChordSymbol>,
}

impl PendingMarks {
    /// Records `marking`, handing a navigation mark back to the caller since
    /// it belongs to the measure rather than to a note.
    fn add(&mut self, marking: Marking) -> Option<Navigation> {
        match marking {
            Marking::Articulation(a) => self.articulations.push(a),
            Marking::Ornament(o) => self.ornaments.push(o),
            Marking::Fermata => self.fermata = true,
            Marking::Dynamic(d) => self.dynamic = Some(d),
            Marking::Wedge(w) => self.wedge = Some(w),
            Marking::Navigation(n) => return Some(n),
            Marking::Ignored => {}
        }
        None
    }

    /// Attaches the per-note marks (articulations/ornaments/fermata) to
    /// `note`, plus - if `first` - the ones a chord carries only once.
    fn apply(&self, note: &mut Note, first: bool) {
        note.articulations
            .extend(self.articulations.iter().cloned());
        note.ornaments.extend(self.ornaments.iter().cloned());
        note.fermata |= self.fermata;
        if first {
            if self.dynamic.is_some() {
                note.dynamic.clone_from(&self.dynamic);
            }
            if self.wedge.is_some() {
                note.wedge = self.wedge;
            }
            if self.chord_symbol.is_some() {
                note.chord_symbol.clone_from(&self.chord_symbol);
            }
        }
    }
}

/// A voice's own bar-by-bar event stream, tracked independently of the
/// other voices until [`MultiVoiceState::finish`] merges them by index.
#[derive(Default)]
struct RawMeasure {
    number: u32,
    events: Vec<Event>,
    barline: Barline,
    navigation: Vec<Navigation>,
}

struct VoiceState {
    measures: Vec<RawMeasure>,
    current: Vec<Event>,
    measure_number: u32,
    /// A `|:` before the measure currently being filled.
    start_repeat: bool,
    /// An ending bracket starting at the measure currently being filled.
    start_ending: Option<Ending>,
    pending_tie: bool,
    tuplet: Option<TupletContext>,
    pending: PendingMarks,
    pending_navigation: Vec<Navigation>,
}

impl VoiceState {
    fn new() -> Self {
        Self {
            measures: vec![RawMeasure::default()],
            current: Vec::new(),
            measure_number: 1,
            start_repeat: false,
            start_ending: None,
            pending_tie: false,
            tuplet: None,
            pending: PendingMarks::default(),
            pending_navigation: Vec::new(),
        }
    }

    fn decoration(&mut self, decoration: &Decoration) {
        if let Some(navigation) = self.pending.add(marking_from_decoration(decoration)) {
            self.pending_navigation.push(navigation);
        }
    }

    fn push_event(&mut self, mut event: Event) {
        if let Some(ctx) = &mut self.tuplet {
            match &mut event {
                Event::Note(n) => n.duration.tuplet = Some(ctx.tuplet),
                Event::Rest(r) => r.duration.tuplet = Some(ctx.tuplet),
                Event::Chord(notes) => {
                    for n in notes.iter_mut() {
                        n.duration.tuplet = Some(ctx.tuplet);
                    }
                }
            }
            ctx.remaining -= 1;
            if ctx.remaining == 0 {
                self.tuplet = None;
            }
        }
        let marks = std::mem::take(&mut self.pending);
        match &mut event {
            Event::Note(n) => marks.apply(n, true),
            Event::Chord(notes) => {
                for (i, n) in notes.iter_mut().enumerate() {
                    marks.apply(n, i == 0);
                }
            }
            Event::Rest(r) => {
                r.fermata = marks.fermata;
                r.chord_symbol = marks.chord_symbol;
            }
        }
        self.current.push(event);
    }

    /// A grace note within a `{...}` group borrows no time and, unlike a
    /// regular note, doesn't consume a tuplet slot or a pending decoration
    /// (both are left for the main note that follows).
    fn push_grace_note(&mut self, mut note: Note) {
        note.grace = true;
        self.current.push(Event::Note(note));
    }

    fn bar(&mut self, text: &str, ending: Option<&str>, at_line_start: bool) {
        let token = parse_bar_token(text, ending);
        // A bar token with no music before it (`|:` opening a tune or a
        // line) only opens the next measure. A plain `|` only counts as
        // such at a line start, so our writer's `[V:2] |` (an empty
        // measure in voice 2) still closes one.
        let opens_something = token.repeat_start || token.ending.is_some();
        let leading = self.current.is_empty()
            && token.repeat_end.is_none()
            && (at_line_start || (self.measures.len() == 1 && opens_something));
        if leading {
            self.start_repeat |= token.repeat_start;
            if token.ending.is_some() {
                self.start_ending = token.ending;
            }
            return;
        }

        let measure = self.measures.last_mut().expect("at least one measure");
        measure.events = std::mem::take(&mut self.current);
        measure.number = self.measure_number;
        measure.barline = Barline {
            kind: token.kind,
            repeat_start: std::mem::take(&mut self.start_repeat),
            repeat_end: token.repeat_end,
            ending: self.start_ending.take(),
        };
        measure.navigation = std::mem::take(&mut self.pending_navigation);
        self.start_repeat = token.repeat_start;
        self.start_ending = token.ending;
        self.measure_number += 1;
        self.measures.push(RawMeasure::default());
    }

    fn finish(mut self) -> Vec<RawMeasure> {
        if self.current.is_empty() && self.pending_navigation.is_empty() {
            self.measures.pop();
        } else {
            let measure = self.measures.last_mut().expect("at least one measure");
            measure.events = self.current;
            measure.number = self.measure_number;
            measure.navigation = self.pending_navigation;
            measure.barline.repeat_start = self.start_repeat;
            measure.barline.ending = self.start_ending;
        }
        self.measures
    }
}

struct MultiVoiceState {
    unit_length: f64,
    measure_quarter_length: f64,
    voices: BTreeMap<u8, VoiceState>,
    current_voice: u8,
    /// Nothing but whitespace/comments seen yet on the current music line.
    at_line_start: bool,
}

impl MultiVoiceState {
    fn new(unit_length: f64, measure_quarter_length: f64) -> Self {
        let mut voices = BTreeMap::new();
        voices.insert(1, VoiceState::new());
        Self {
            unit_length,
            measure_quarter_length,
            voices,
            current_voice: 1,
            at_line_start: true,
        }
    }

    fn voice(&mut self) -> &mut VoiceState {
        self.voices
            .entry(self.current_voice)
            .or_insert_with(VoiceState::new)
    }

    fn process_body(&mut self, body: &TuneBody) {
        for line in &body.lines {
            if let TuneLine::Music(MusicLine { symbols }) = line {
                self.at_line_start = true;
                self.process_symbols(symbols);
            }
        }
    }

    fn process_symbols(&mut self, symbols: &[MusicSymbol]) {
        for symbol in symbols {
            self.process_symbol(symbol);
        }
    }

    fn process_symbol(&mut self, symbol: &MusicSymbol) {
        let at_line_start = self.at_line_start;
        if !matches!(symbol, MusicSymbol::Space(_) | MusicSymbol::Comment(_)) {
            self.at_line_start = false;
        }
        match symbol {
            MusicSymbol::Note {
                accidental,
                note,
                octave,
                length,
                tie,
            } => {
                let length_mult = length.as_ref().map(length_value).unwrap_or(1.0);
                let unit_length = self.unit_length;
                let n = self.voice().build_note(
                    *accidental,
                    *note,
                    *octave,
                    length_mult,
                    *tie,
                    unit_length,
                );
                self.voice().push_event(Event::Note(n));
            }
            MusicSymbol::Chord { notes, length, tie } => {
                let outer_mult = length.as_ref().map(length_value).unwrap_or(1.0);
                let unit_length = self.unit_length;
                let mut built = Vec::new();
                // Decorations inside the brackets apply to the next inner note.
                let mut inner_marks = PendingMarks::default();
                for inner in notes {
                    match inner {
                        MusicSymbol::Note {
                            accidental,
                            note,
                            octave,
                            length: inner_length,
                            tie: inner_tie,
                        } => {
                            let inner_mult = inner_length.as_ref().map(length_value).unwrap_or(1.0);
                            let mut n = self.voice().build_note(
                                *accidental,
                                *note,
                                *octave,
                                inner_mult * outer_mult,
                                inner_tie.or(*tie),
                                unit_length,
                            );
                            std::mem::take(&mut inner_marks).apply(&mut n, true);
                            built.push(n);
                        }
                        MusicSymbol::Decoration(decoration) => {
                            if let Some(navigation) =
                                inner_marks.add(marking_from_decoration(decoration))
                            {
                                self.voice().pending_navigation.push(navigation);
                            }
                        }
                        _ => {}
                    }
                }
                self.voice().push_event(Event::Chord(built));
            }
            MusicSymbol::Rest(rest) => {
                let measure_quarter_length = self.measure_quarter_length;
                let unit_length = self.unit_length;
                let duration = rest_duration(rest, unit_length, measure_quarter_length);
                self.voice().push_event(Event::Rest(Rest::new(duration)));
            }
            MusicSymbol::Bar(text, ending) => {
                self.voice().bar(text, ending.as_deref(), at_line_start)
            }
            MusicSymbol::Ending(spec) => self.voice().start_ending = Some(ending_start(spec)),
            MusicSymbol::Tuplet { p, q, r } => {
                let normal = q.unwrap_or_else(|| default_tuplet_normal(*p));
                let count = r.unwrap_or(*p);
                self.voice().tuplet = Some(TupletContext {
                    remaining: count,
                    tuplet: Tuplet {
                        actual: *p as u8,
                        normal: normal as u8,
                    },
                });
            }
            MusicSymbol::BrokenRhythm {
                rhythm,
                before,
                after,
            } => {
                let before_start = self.voice().current.len();
                self.process_symbols(before);
                let before_end = self.voice().current.len();

                let after_start = self.voice().current.len();
                self.process_symbols(after);
                let after_end = self.voice().current.len();

                let n = (rhythm.len().clamp(1, 3)) as i32;
                let long_mult = 2.0 - 0.5f64.powi(n);
                let short_mult = 0.5f64.powi(n);
                let (first_mult, second_mult) = if rhythm.starts_with('>') {
                    (long_mult, short_mult)
                } else {
                    (short_mult, long_mult)
                };

                let voice = self.voice();
                for event in &mut voice.current[before_start..before_end] {
                    scale_event_duration(event, first_mult);
                }
                for event in &mut voice.current[after_start..after_end] {
                    scale_event_duration(event, second_mult);
                }
            }
            MusicSymbol::Decoration(decoration) => self.voice().decoration(decoration),
            MusicSymbol::Annotation(annotation) => {
                if let Some(symbol) = chord_symbol_from_annotation(annotation) {
                    self.voice().pending.chord_symbol = Some(symbol);
                }
            }
            MusicSymbol::InlineField(InfoField(field, value), _) if *field == 'V' => {
                if let Some(number) = value.split_whitespace().next().and_then(|s| s.parse().ok()) {
                    self.current_voice = number;
                    self.voices.entry(number).or_insert_with(VoiceState::new);
                }
            }
            MusicSymbol::GraceNotes { notes, .. } => {
                for inner in notes {
                    if let MusicSymbol::Note {
                        accidental,
                        note,
                        octave,
                        length,
                        tie,
                    } = inner
                    {
                        let length_mult = length.as_ref().map(length_value).unwrap_or(1.0);
                        let unit_length = self.unit_length;
                        let n = self.voice().build_note(
                            *accidental,
                            *note,
                            *octave,
                            length_mult,
                            *tie,
                            unit_length,
                        );
                        self.voice().push_grace_note(n);
                    }
                }
            }
            // Ornamental/other symbols that don't map onto our model.
            MusicSymbol::Beam(_)
            | MusicSymbol::Slur(_)
            | MusicSymbol::Comment(_)
            | MusicSymbol::Spacer
            | MusicSymbol::InlineField(_, _)
            | MusicSymbol::Space(_)
            | MusicSymbol::Reserved(_) => {}
        }
    }

    /// Merges each voice's independently-tracked measures into shared
    /// `Measure`s (by index), using voice 1's barline/navigation as the
    /// measure-level marking when voices disagree.
    fn finish(self) -> Vec<Measure> {
        let per_voice: Vec<(u8, Vec<RawMeasure>)> = self
            .voices
            .into_iter()
            .map(|(n, v)| (n, v.finish()))
            .collect();
        let measure_count = per_voice.iter().map(|(_, m)| m.len()).max().unwrap_or(0);

        (0..measure_count)
            .map(|i| {
                let voices = per_voice
                    .iter()
                    .map(|(number, measures)| Voice {
                        number: *number,
                        events: measures
                            .get(i)
                            .map(|m| m.events.clone())
                            .unwrap_or_default(),
                    })
                    .collect();
                let (barline, navigation, number) = per_voice
                    .first()
                    .and_then(|(_, measures)| measures.get(i))
                    .map(|m| (m.barline.clone(), m.navigation.clone(), m.number))
                    .unwrap_or_else(|| (Barline::default(), Vec::new(), (i + 1) as u32));
                Measure {
                    number,
                    attributes: MeasureAttributes::default(),
                    voices,
                    barline,
                    navigation,
                }
            })
            .collect()
    }
}

impl VoiceState {
    fn build_note(
        &mut self,
        accidental: Option<Accidental>,
        note: AbcNote,
        octave: i8,
        length_mult: f64,
        tie: Option<AbcTie>,
        unit_length: f64,
    ) -> Note {
        let step = to_step(note);
        let alter = accidental.map(alter_from_accidental).unwrap_or(0);
        // abc_parser's octave is 1 for the uppercase row, 2 for lowercase;
        // scientific pitch notation puts middle C (uppercase, no marks) at 4.
        let scientific_octave = octave + 3;
        let duration = Duration::from_quarter_length_quantized(unit_length * length_mult);

        let starts_tie = tie.is_some();
        let stops_tie = self.pending_tie;
        let model_tie = match (stops_tie, starts_tie) {
            (true, true) => Tie::StartStop,
            (true, false) => Tie::Stop,
            (false, true) => Tie::Start,
            (false, false) => Tie::None,
        };
        self.pending_tie = starts_tie;

        Note::new(
            Pitch::new(step, alter, scientific_octave),
            duration,
            model_tie,
        )
    }
}

fn rest_duration(rest: &AbcRest, unit_length: f64, measure_quarter_length: f64) -> Duration {
    match rest {
        AbcRest::Note(len) | AbcRest::NoteHidden(len) => {
            let mult = len.as_ref().map(length_value).unwrap_or(1.0);
            Duration::from_quarter_length_quantized(unit_length * mult)
        }
        AbcRest::Measure(len) | AbcRest::MeasureHidden(len) => {
            let mult = len.as_ref().map(length_value).unwrap_or(1.0);
            Duration::from_quarter_length_quantized(measure_quarter_length * mult)
        }
    }
}

fn scale_event_duration(event: &mut Event, mult: f64) {
    match event {
        Event::Note(n) => n.duration = requantize_scaled(n.duration, mult),
        Event::Rest(r) => r.duration = requantize_scaled(r.duration, mult),
        Event::Chord(notes) => {
            for n in notes.iter_mut() {
                n.duration = requantize_scaled(n.duration, mult);
            }
        }
    }
}

fn requantize_scaled(duration: Duration, mult: f64) -> Duration {
    let tuplet = duration.tuplet;
    let mut scaled = Duration::from_quarter_length_quantized(duration.quarter_length() * mult);
    scaled.tuplet = tuplet;
    scaled
}
