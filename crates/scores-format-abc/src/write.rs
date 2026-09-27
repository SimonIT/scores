//! Builds an [`abc_parser`] parse tree from a [`Score`] and serializes it
//! with the crate's own [`ToABC`] writer, rather than formatting ABC text by
//! hand. Each [`Part`] becomes one tune (`X:1`, `X:2`, ...); accidentals are
//! only ever written explicitly (never as a `=` cancelling the key
//! signature).
//!
//! A part with more than one voice gets a `V:n` header line per voice, and
//! its body is written one voice at a time (`[V:1]` then all of voice 1's
//! measures, then `[V:2]` and all of voice 2's, ...) rather than
//! system-by-system - not idiomatic ABC layout, but round-trips correctly
//! through our own reader.
//!
//! Before each note/chord/rest come its chord symbol (`"Am7"`) and then its
//! decorations: fermata, articulations, ornaments, dynamic (`!mf!`) and
//! hairpin (`!<(!`, `!>(!`, and `!<)!`/`!>)!` matching whichever was open).
//! A chord's marks are taken from its first note. Marks ABC has no
//! decoration for (e.g. `Articulation::Doit`, `Ornament::Schleifer`,
//! `Dynamic::Other`) are dropped; `Other` articulations/ornaments are
//! written verbatim as `!name!`. A rest keeps only its fermata and chord
//! symbol. Segno/Coda signs are written at the start of their measure and
//! jumps (`!D.C.!`, `!D.S.alcoda!`, `!fine!`, `!dacoda!`, ...) at its end.
//!
//! Each bar token carries the closing measure's style (`|`, `||`, `|]`,
//! `[|`, `.|`, `[|]`; dashed becomes dotted, heavy/heavy-heavy become `|]`)
//! and `:|`, plus the next measure's `|:` and ending number (`|1`, or
//! `|[1,2` for a list); a repeat/ending at the very first measure gets a
//! leading bar token. ABC has no end-of-ending token, so an ending's extent
//! and closed/open hook aren't written; the reader infers them (see
//! the `read` module). A repeat count above 2 isn't expressible (`:|` means
//! twice). A run of consecutive grace notes becomes one `{...}` group;
//! their own marks aren't written, since ABC's grammar doesn't allow
//! decorating a note inside a grace group.

use abc_parser::datatypes::writer::ToABC;
use abc_parser::datatypes::{
    Accidental as AbcAccidental, Annotation, Decoration, HeaderLine, InfoField,
    Length as AbcLength, MusicLine, MusicSymbol, Note as AbcNote, Rest as AbcRest, Tie as AbcTie,
    Tune, TuneBody, TuneBook, TuneHeader, TuneLine,
};
use scores_core::{
    Articulation, Barline, BarlineType, ChordSymbol, Duration, Event, KeySignature, Measure, Mode,
    Navigation, Note, Ornament, Part, Result, Score, Step, Tie, Wedge,
};

/// Unit note length (`L:`) every tune is written with, in quarter notes.
const UNIT_LENGTH: f64 = 0.5;

pub fn write(score: &Score) -> Result<Vec<u8>> {
    let tunes = score
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| write_tune(score, part, index + 1))
        .collect();

    let tune_book = TuneBook::new(Vec::new(), None, tunes);
    Ok(tune_book.to_abc().into_bytes())
}

fn write_tune(score: &Score, part: &Part, index: usize) -> Tune {
    let first_attrs = part.measures.first().map(|m| &m.attributes);
    let time_signature = first_attrs
        .and_then(|a| a.time_signature)
        .unwrap_or_default();
    let key_signature = first_attrs
        .and_then(|a| a.key_signature)
        .unwrap_or_default();

    let mut header_lines = vec![HeaderLine::Field(InfoField('X', index.to_string()), None)];

    let title = part
        .name
        .clone()
        .or_else(|| (index == 1).then(|| score.title.clone()).flatten());
    if let Some(title) = title {
        header_lines.push(HeaderLine::Field(InfoField('T', title), None));
    }
    if index == 1
        && let Some(composer) = &score.composer
    {
        header_lines.push(HeaderLine::Field(InfoField('C', composer.clone()), None));
    }
    header_lines.push(HeaderLine::Field(
        InfoField(
            'M',
            format!("{}/{}", time_signature.beats, time_signature.beat_type),
        ),
        None,
    ));
    header_lines.push(HeaderLine::Field(InfoField('L', "1/8".to_string()), None));
    header_lines.push(HeaderLine::Field(
        InfoField('K', key_to_abc(&key_signature)),
        None,
    ));

    let voice_numbers = collect_voice_numbers(part);
    if voice_numbers.len() > 1 {
        for &voice_number in &voice_numbers {
            header_lines.push(HeaderLine::Field(
                InfoField('V', voice_number.to_string()),
                None,
            ));
        }
    }

    let body_lines = write_body(part, &voice_numbers);
    Tune::new(
        TuneHeader::new(header_lines),
        Some(TuneBody::new(body_lines)),
    )
}

fn collect_voice_numbers(part: &Part) -> Vec<u8> {
    let mut numbers: Vec<u8> = part
        .measures
        .iter()
        .flat_map(|m| m.voices.iter().map(|v| v.number))
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    if numbers.is_empty() {
        numbers.push(1);
    }
    numbers
}

fn key_to_abc(key: &KeySignature) -> String {
    match key.mode {
        Mode::Major => crate::fifths_to_tonic(key.fifths).to_string(),
        Mode::Minor => format!("{}m", crate::fifths_to_tonic_minor(key.fifths)),
    }
}

/// Per-voice state carried across measures while writing.
#[derive(Default)]
struct VoiceWriter {
    /// The hairpin currently open, so its [`Wedge::Stop`] can be written
    /// with the matching `!<)!`/`!>)!`.
    open_wedge: Option<Wedge>,
}

fn write_body(part: &Part, voice_numbers: &[u8]) -> Vec<TuneLine> {
    let mut symbols = Vec::new();
    let multi_voice = voice_numbers.len() > 1;
    let measures = &part.measures;

    for &voice_number in voice_numbers {
        let mut writer = VoiceWriter::default();
        if multi_voice {
            symbols.push(MusicSymbol::InlineField(
                InfoField('V', voice_number.to_string()),
                true,
            ));
            symbols.push(MusicSymbol::Space(" ".to_string()));
        }
        // A repeat/ending starting at the very first measure needs a
        // leading bar token (`|:`, `|1`) - every later one is written on
        // the bar closing the previous measure.
        if let Some(first) = measures.first()
            && (first.barline.repeat_start || ending_starting_at(measures, 0).is_some())
        {
            let token = if first.barline.repeat_start {
                "|:"
            } else {
                "|"
            };
            push_bar(
                token.to_string(),
                ending_starting_at(measures, 0),
                &mut symbols,
            );
            symbols.push(MusicSymbol::Space(" ".to_string()));
        }
        for (index, measure) in measures.iter().enumerate() {
            if index > 0 {
                symbols.push(MusicSymbol::Space(" ".to_string()));
            }
            let (opening, closing) = navigation_decorations(&measure.navigation);
            symbols.extend(opening.into_iter().map(MusicSymbol::Decoration));
            if let Some(voice) = measure.voices.iter().find(|v| v.number == voice_number) {
                writer.write_events(&voice.events, &mut symbols);
            }
            symbols.extend(closing.into_iter().map(MusicSymbol::Decoration));
            let next = measures.get(index + 1);
            let token = barline_token(&measure.barline, next.map(|m| &m.barline), next.is_none());
            push_bar(token, ending_starting_at(measures, index + 1), &mut symbols);
        }
        if multi_voice {
            symbols.push(MusicSymbol::Space(" ".to_string()));
        }
    }

    vec![TuneLine::Music(MusicLine::new(symbols))]
}

/// The ending numbers to write in front of `measures[index]`, if an ending
/// bracket starts there. ABC has no end-of-ending token, so the bracket's
/// extent and closedness aren't written (the reader infers them).
fn ending_starting_at(measures: &[Measure], index: usize) -> Option<&[u8]> {
    let ending = measures.get(index)?.barline.ending.as_ref()?;
    (!ending.numbers.is_empty()).then_some(ending.numbers.as_slice())
}

/// `abc_parser`'s bar token only takes a single ending number/range after
/// it (`|1`, `|1-3`), so a list is written as a separate `[1,2` ending.
fn push_bar(token: String, ending: Option<&[u8]>, symbols: &mut Vec<MusicSymbol>) {
    match ending {
        Some([number]) => symbols.push(MusicSymbol::Bar(token, Some(number.to_string()))),
        Some(numbers) => {
            symbols.push(MusicSymbol::Bar(token, None));
            let list = numbers.iter().map(u8::to_string).collect::<Vec<_>>();
            symbols.push(MusicSymbol::Ending(list.join(",")));
        }
        None => symbols.push(MusicSymbol::Bar(token, None)),
    }
}

/// A measure's navigation marks as decorations: Segno/Coda signs are
/// written at the start of the measure (the first list), jump instructions
/// (D.C., Fine, ...) at its end, just before the closing bar (the second).
fn navigation_decorations(navigation: &[Navigation]) -> (Vec<Decoration>, Vec<Decoration>) {
    let unresolved = |name: &str| Decoration::Unresolved(name.to_string());
    let mut opening = Vec::new();
    let mut closing = Vec::new();
    for mark in navigation {
        match mark {
            Navigation::Segno => opening.push(Decoration::Segno),
            Navigation::Coda => opening.push(Decoration::Coda),
            Navigation::DaCapo => closing.push(unresolved("D.C.")),
            Navigation::DaCapoAlFine => closing.push(unresolved("D.C.alfine")),
            Navigation::DaCapoAlCoda => closing.push(unresolved("D.C.alcoda")),
            Navigation::DalSegno => closing.push(unresolved("D.S.")),
            Navigation::DalSegnoAlFine => closing.push(unresolved("D.S.alfine")),
            Navigation::DalSegnoAlCoda => closing.push(unresolved("D.S.alcoda")),
            Navigation::Fine => closing.push(unresolved("fine")),
            Navigation::ToCoda => closing.push(unresolved("dacoda")),
        }
    }
    (opening, closing)
}

/// The bar token closing a measure: its own style and repeat-end mark
/// (`:|`, a prefix) plus the next measure's repeat-start mark (`|:`, a
/// suffix); both may combine (`:|:`). The very last measure of a tune
/// closes with a final bar unless it asks for a specific other style.
/// Styles ABC can't draw fall back to the closest one it can.
fn barline_token(barline: &Barline, next: Option<&Barline>, is_last: bool) -> String {
    let kind = if is_last && barline.kind == BarlineType::Regular {
        BarlineType::LightHeavy
    } else {
        barline.kind
    };
    let (dotted, core) = match kind {
        BarlineType::Regular | BarlineType::Tick | BarlineType::Short => (false, "|"),
        BarlineType::Dotted | BarlineType::Dashed => (true, "|"),
        BarlineType::LightLight => (false, "||"),
        BarlineType::LightHeavy | BarlineType::Heavy | BarlineType::HeavyHeavy => (false, "|]"),
        BarlineType::HeavyLight => (false, "[|"),
        BarlineType::NoBarline => (false, "[|]"),
    };
    let mut token = String::new();
    if dotted {
        token.push('.');
    }
    if barline.repeat_end.is_some() {
        token.push(':');
    }
    token.push_str(core);
    if next.is_some_and(|b| b.repeat_start) {
        token.push(':');
    }
    token
}

impl VoiceWriter {
    fn write_events(&mut self, events: &[Event], symbols: &mut Vec<MusicSymbol>) {
        let mut i = 0;
        while i < events.len() {
            if i > 0 {
                symbols.push(MusicSymbol::Space(" ".to_string()));
            }
            if is_grace_note(&events[i]) {
                let run_len = events[i..].iter().take_while(|e| is_grace_note(e)).count();
                symbols.push(grace_notes_symbol(&events[i..i + run_len]));
                i += run_len;
                continue;
            }
            match events[i].duration().tuplet {
                Some(tuplet) => {
                    let run_len = events[i..]
                        .iter()
                        .take_while(|e| e.duration().tuplet == Some(tuplet))
                        .count();
                    symbols.push(MusicSymbol::Tuplet {
                        p: tuplet.actual as u32,
                        q: Some(tuplet.normal as u32),
                        r: Some(run_len as u32),
                    });
                    for event in &events[i..i + run_len] {
                        self.push_event_symbols(event, symbols);
                    }
                    i += run_len;
                }
                None => {
                    self.push_event_symbols(&events[i], symbols);
                    i += 1;
                }
            }
        }
    }

    fn push_event_symbols(&mut self, event: &Event, symbols: &mut Vec<MusicSymbol>) {
        match event {
            Event::Note(note) => {
                self.push_markers(note, symbols);
                symbols.push(note_to_symbol(note));
            }
            Event::Rest(rest) => {
                push_chord_symbol(rest.chord_symbol.as_ref(), symbols);
                if rest.fermata {
                    symbols.push(MusicSymbol::Decoration(Decoration::Fermata));
                }
                let length = length_for(&rest.duration);
                symbols.push(MusicSymbol::Rest(AbcRest::Note(length)));
            }
            Event::Chord(notes) => {
                if let Some(first) = notes.first() {
                    self.push_markers(first, symbols);
                }
                let inner = notes.iter().map(note_to_symbol).collect();
                symbols.push(MusicSymbol::Chord {
                    notes: inner,
                    length: None,
                    tie: None,
                });
            }
        }
    }

    /// Writes a note's chord symbol then its decorations - the order ABC
    /// 2.1 prescribes before a note (after any grace notes).
    fn push_markers(&mut self, note: &Note, symbols: &mut Vec<MusicSymbol>) {
        push_chord_symbol(note.chord_symbol.as_ref(), symbols);
        let mut decorate = |decoration| symbols.push(MusicSymbol::Decoration(decoration));
        if note.fermata {
            decorate(Decoration::Fermata);
        }
        for articulation in &note.articulations {
            if let Some(decoration) = decoration_from_articulation(articulation) {
                decorate(decoration);
            }
        }
        for ornament in &note.ornaments {
            decoration_from_ornament(ornament, &mut decorate);
        }
        if let Some(name) = note.dynamic.as_ref().and_then(crate::dynamic_name) {
            decorate(Decoration::Unresolved(name.to_string()));
        }
        if let Some(wedge) = note.wedge {
            let name = match wedge {
                Wedge::CrescendoStart => "<(",
                Wedge::DiminuendoStart => ">(",
                Wedge::Stop if self.open_wedge == Some(Wedge::DiminuendoStart) => ">)",
                Wedge::Stop => "<)",
            };
            self.open_wedge = (wedge != Wedge::Stop).then_some(wedge);
            decorate(Decoration::Unresolved(name.to_string()));
        }
    }
}

/// An ABC chord symbol is a quoted annotation with no placement prefix.
fn push_chord_symbol(chord_symbol: Option<&ChordSymbol>, symbols: &mut Vec<MusicSymbol>) {
    if let Some(chord_symbol) = chord_symbol {
        symbols.push(MusicSymbol::Annotation(Annotation::new(
            None,
            chord_symbol.to_string(),
        )));
    }
}

fn is_grace_note(event: &Event) -> bool {
    matches!(event, Event::Note(n) if n.grace)
}

/// Groups a run of consecutive grace notes into one `{...}` (never
/// acciaccatura-marked, since our model doesn't distinguish it).
fn grace_notes_symbol(events: &[Event]) -> MusicSymbol {
    let notes = events
        .iter()
        .filter_map(|e| match e {
            Event::Note(n) => Some(note_to_symbol(n)),
            _ => None,
        })
        .collect();
    MusicSymbol::GraceNotes {
        acciaccatura: None,
        notes,
    }
}

/// A free-text `Other` mark is written as `!name!` unless that would break
/// the decoration syntax.
fn is_writable_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['!', '\n'])
}

fn decoration_from_articulation(articulation: &Articulation) -> Option<Decoration> {
    let unresolved = |name: &str| Some(Decoration::Unresolved(name.to_string()));
    match articulation {
        Articulation::Staccato => Some(Decoration::Staccato),
        Articulation::Accent => Some(Decoration::Accent),
        Articulation::StrongAccent => unresolved("marcato"),
        Articulation::Tenuto => unresolved("tenuto"),
        Articulation::Staccatissimo => unresolved("wedge"),
        Articulation::BreathMark => unresolved("breath"),
        Articulation::Scoop => unresolved("slide"),
        Articulation::Other(s) if s == "up-bow" => Some(Decoration::UpBow),
        Articulation::Other(s) if s == "down-bow" => Some(Decoration::DownBow),
        Articulation::Other(s) if is_writable_name(s) => unresolved(s),
        _ => None,
    }
}

/// Emits the decoration(s) for an ornament; a wavy line is written as a
/// `!trill(!`...`!trill)!` pair spanning just this note.
fn decoration_from_ornament(ornament: &Ornament, decorate: &mut impl FnMut(Decoration)) {
    let unresolved = |name: &str| Decoration::Unresolved(name.to_string());
    match ornament {
        Ornament::TrillMark => decorate(Decoration::Trill),
        Ornament::Mordent => decorate(Decoration::LowerMordent),
        Ornament::InvertedMordent => decorate(Decoration::UpperMordent),
        Ornament::Turn => decorate(unresolved("turn")),
        Ornament::InvertedTurn => decorate(unresolved("invertedturn")),
        Ornament::WavyLine => {
            decorate(unresolved("trill("));
            decorate(unresolved("trill)"));
        }
        Ornament::Tremolo(marks @ 1..=4) => decorate(unresolved(&"/".repeat(*marks as usize))),
        Ornament::Other(s) if s == "roll" => decorate(Decoration::Roll),
        Ornament::Other(s) if is_writable_name(s) => decorate(unresolved(s)),
        _ => {}
    }
}

fn note_to_symbol(note: &Note) -> MusicSymbol {
    let crate_octave = note.pitch.octave - 3;
    let abc_note = to_abc_note(note.pitch.step);
    let accidental = accidental_from_alter(note.pitch.alter);
    let length = length_for(&note.duration);
    let tie = matches!(note.tie, Tie::Start | Tie::StartStop).then_some(AbcTie::Solid);
    MusicSymbol::new_note(accidental, abc_note, crate_octave, length, tie)
}

/// The note/rest's own written length (base value + dots, ignoring any
/// tuplet ratio - that is instead conveyed by a preceding `MusicSymbol::Tuplet`
/// marker), as an ABC length multiplier relative to [`UNIT_LENGTH`].
fn length_for(duration: &Duration) -> Option<AbcLength> {
    let mut written = duration.base.quarter_length();
    let mut addend = written;
    for _ in 0..duration.dots {
        addend /= 2.0;
        written += addend;
    }
    let multiplier = (written / UNIT_LENGTH) as f32;
    if (multiplier - 1.0).abs() < f32::EPSILON {
        None
    } else {
        Some(AbcLength::new(multiplier))
    }
}

fn to_abc_note(step: Step) -> AbcNote {
    match step {
        Step::A => AbcNote::A,
        Step::B => AbcNote::B,
        Step::C => AbcNote::C,
        Step::D => AbcNote::D,
        Step::E => AbcNote::E,
        Step::F => AbcNote::F,
        Step::G => AbcNote::G,
    }
}

fn accidental_from_alter(alter: i8) -> Option<AbcAccidental> {
    match alter {
        2 => Some(AbcAccidental::DoubleSharp),
        1 => Some(AbcAccidental::Sharp),
        -1 => Some(AbcAccidental::Flat),
        -2 => Some(AbcAccidental::DoubleFlat),
        _ => None,
    }
}
