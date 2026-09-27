use std::collections::BTreeSet;

use primus_scores::measures::{
    Barline as PriBarline, HeaderTexts, MeasureSpec, MeasureText, RepeatEnding, Tempo as PriTempo,
    TempoMark, TimeSignature as PriTimeSignature,
};
use primus_scores::writer::{
    Chord, Drums, Element, ElementRef, Grace, Measure as PriMeasure, Note as PriNote,
    Rest as PriRest, Span, Value,
};
use primus_scores::{ClefChange, NoteSymbol, PriError, PriFile, Tremolo};
use scores_core::{
    Clef, Duration, Event, Measure, Navigation, Note, Ornament, Part, Result, Score, ScoresError,
    Tie, Wedge,
};

use crate::convert::{
    articulation_kinds, barline_kind_to_primus, clef_to_primus, dynamic_kind, ending_label, kind,
    navigation_text, ornament_kind, rests_filling, staff_step_from_pitch, value_from_duration,
    windows_1252,
};

/// The document new scores are written into: PriMus Free's "Singstimme"
/// template (one treble staff, one voice), for its fonts, page layout and
/// staff settings. Further parts are copies of its staff.
const TEMPLATE: &[u8] = include_bytes!("../templates/Singstimme.pri");

/// Measures per system; PriMus reflows the systems on load.
const MEASURES_PER_SYSTEM: usize = 4;

/// Most symbols PriMus takes on one note.
const MAX_SYMBOLS: usize = 8;

/// Longest text PriMus takes, in bytes.
const MAX_TEXT: usize = 255;

fn write_error(e: PriError) -> ScoresError {
    ScoresError::Write(e.to_string())
}

pub fn write(score: &Score) -> Result<Vec<u8>> {
    if score.parts.is_empty() {
        return Err(ScoresError::Unsupported(
            "cannot write an empty score to PriMus format".into(),
        ));
    }
    let mut file = PriFile::from_bytes(TEMPLATE).map_err(write_error)?;
    let doc = &mut file.document;
    let measure_count = score
        .parts
        .iter()
        .map(|p| p.measures.len())
        .max()
        .unwrap_or(0)
        .max(1);

    // The template's staff plays a choir "ooh"; start from a piano sound,
    // which every added staff copies.
    if let Some(mut midi) = doc.instrument_midi(0) {
        midi.program = 0;
        doc.set_instrument_midi(0, midi);
    }
    for p in 1..score.parts.len() {
        doc.add_staff(p - 1).map_err(write_error)?;
    }
    let mut part_slots = Vec::new();
    for (p, part) in score.parts.iter().enumerate() {
        let clef = part
            .measures
            .iter()
            .find_map(|m| m.attributes.clef)
            .unwrap_or_default();
        if clef == Clef::Percussion {
            doc.set_instrument_percussion(p);
        } else {
            doc.set_instrument_clef(p, clef_to_primus(clef));
        }
        if let (Some(name), Some(mut names)) = (&part.name, doc.instrument_names(p)) {
            names.long = text(name);
            doc.set_instrument_names(p, &names);
        }
        let numbers: BTreeSet<u8> = part
            .measures
            .iter()
            .flat_map(|m| m.voices.iter().map(|v| v.number))
            .collect();
        let slots: Vec<Option<u8>> = if numbers.is_empty() {
            vec![None]
        } else {
            numbers.into_iter().map(Some).collect()
        };
        for _ in 1..slots.len() {
            doc.add_voice(p).map_err(write_error)?;
        }
        part_slots.push(slots);
    }

    let specs = measure_specs(score, measure_count);
    doc.set_measures(&specs, MEASURES_PER_SYSTEM)
        .map_err(write_error)?;

    // The segno goes on the first note of its measure where there is one.
    let segnos: Vec<bool> = (0..measure_count)
        .map(|m| {
            shared_measure(score, m).is_some_and(|s| s.navigation.contains(&Navigation::Segno))
        })
        .collect();
    let mut segno_placed = vec![false; measure_count];

    // Every voice's elements, then the measures' lengths from all of them.
    let mut voices: Vec<(usize, Vec<BuiltMeasure>)> = Vec::new();
    for (p, part) in score.parts.iter().enumerate() {
        let staff_voices: Vec<usize> = (0..doc.voice_count())
            .filter(|&v| doc.voice_instrument(v) == Some(p))
            .collect();
        let percussion = part
            .measures
            .iter()
            .find_map(|m| m.attributes.clef)
            .is_some_and(|c| c == Clef::Percussion);
        for (k, &number) in part_slots[p].iter().enumerate() {
            let Some(&v) = staff_voices.get(k) else {
                return Err(ScoresError::Write(format!(
                    "part {}: voice {k} was not created",
                    p + 1
                )));
            };
            let mut built: Vec<BuiltMeasure> = (0..measure_count)
                .map(|m| build_measure(voice_events(part, m, number), percussion))
                .collect();
            if p == 0 && k == 0 {
                for (m, b) in built.iter_mut().enumerate() {
                    if segnos[m] && b.add_segno() {
                        segno_placed[m] = true;
                    }
                }
            }
            voices.push((v, built));
        }
    }
    // PriMus keeps a measure's actual length apart from its time signature
    // (pickups, shortened measures at a hymn's phrase ends, a scale written
    // out in one long measure): each measure is as long as its longest
    // voice, an empty one as its time signature.
    let lengths: Vec<u32> = (0..measure_count)
        .map(|m| {
            voices
                .iter()
                .map(|(_, built)| built[m].ticks())
                .max()
                .filter(|&t| t > 0)
                .unwrap_or_else(|| specs[m].time.ticks().unwrap_or(3840))
        })
        .collect();

    let mut written = BTreeSet::new();
    for (v, built) in voices {
        let (measures, wedges) = fit_voice(built, &lengths)?;
        let spans = wedge_spans(&wedges, &measures);
        if doc.write_voice_with_spans(v, &measures, &spans).is_err() {
            doc.write_voice(v, &measures).map_err(write_error)?;
        }
        written.insert(v);
    }
    for v in (0..doc.voice_count()).filter(|v| !written.contains(v)) {
        let empty: Vec<PriMeasure> = lengths
            .iter()
            .map(|&ticks| PriMeasure {
                ticks,
                elements: Vec::new(),
            })
            .collect();
        doc.write_voice(v, &empty).map_err(write_error)?;
    }

    for (p, part) in score.parts.iter().enumerate() {
        let mut current = part.measures.iter().find_map(|m| m.attributes.clef);
        let mut changes = Vec::new();
        for (m, measure) in part.measures.iter().enumerate().skip(1) {
            if let Some(clef) = measure.attributes.clef
                && Some(clef) != current
            {
                changes.push(ClefChange {
                    measure: m as u32,
                    tick: 0,
                    clef: clef_to_primus(clef),
                });
                current = Some(clef);
            }
        }
        if !changes.is_empty() {
            doc.set_instrument_clef_changes(p, &changes);
        }
    }

    let title = score.title.as_deref().map(text);
    doc.set_title(title.clone().unwrap_or_default());
    doc.set_header_texts(&HeaderTexts {
        title,
        composer: score.composer.as_deref().map(text),
        ..HeaderTexts::default()
    })
    .map_err(write_error)?;

    let mut marks = Vec::new();
    let mut current_bpm = None;
    for m in 0..measure_count {
        let Some(tempo) = score
            .parts
            .iter()
            .find_map(|p| p.measures.get(m).and_then(|m| m.attributes.tempo))
        else {
            continue;
        };
        if m == 0
            && let Some(t) = PriTempo::from_bpm(f64::from(tempo.bpm))
        {
            doc.set_tempo(t).map_err(write_error)?;
        }
        let bpm = tempo.bpm.round() as u32;
        if bpm > 0 && current_bpm != Some(bpm) {
            marks.push(TempoMark::new(m, 4, false, bpm));
            current_bpm = Some(bpm);
        }
    }
    if !marks.is_empty() {
        doc.set_tempo_marks(&marks).map_err(write_error)?;
    }

    let endings: Vec<RepeatEnding> = (0..measure_count)
        .filter_map(|m| {
            let ending = shared_measure(score, m)?.barline.ending.as_ref()?;
            let last = (m + ending.measures.max(1) as usize - 1).min(measure_count - 1);
            Some(RepeatEnding {
                first: u16::try_from(m).ok()?,
                last: u16::try_from(last).ok()?,
                text: ending_label(&ending.numbers),
                open: !ending.closed,
            })
        })
        .collect();
    if !endings.is_empty() {
        doc.set_repeat_endings(&endings).map_err(write_error)?;
    }

    let texts: Vec<MeasureText> = (0..measure_count)
        .flat_map(|m| {
            let navigation = shared_measure(score, m).map_or(&[][..], |s| &s.navigation[..]);
            navigation
                .iter()
                .filter(|&&n| !(n == Navigation::Segno && segno_placed[m]))
                .map(move |&n| MeasureText::above(m, navigation_text(n)))
                .collect::<Vec<_>>()
        })
        .collect();
    if !texts.is_empty() {
        doc.set_measure_texts(&texts).map_err(write_error)?;
    }

    file.to_bytes().map_err(write_error)
}

/// A text as PriMus can store it: Windows-1252, at most 255 bytes.
fn text(s: &str) -> String {
    windows_1252(s).chars().take(MAX_TEXT).collect()
}

/// Measure `m` of the first part that has it: where the barline, repeats,
/// endings and navigation shared by all staves are taken from.
fn shared_measure(score: &Score, m: usize) -> Option<&Measure> {
    score.parts.iter().find_map(|p| p.measures.get(m))
}

fn measure_specs(score: &Score, count: usize) -> Vec<MeasureSpec> {
    let mut time = PriTimeSignature::new(4, 4);
    let mut key = 0i8;
    (0..count)
        .map(|m| {
            if let Some(t) = score
                .parts
                .iter()
                .find_map(|p| p.measures.get(m)?.attributes.time_signature)
            {
                let candidate = PriTimeSignature::new(t.beats, t.beat_type);
                if candidate.ticks().is_some() {
                    time = candidate;
                }
            }
            if let Some(k) = score
                .parts
                .iter()
                .find_map(|p| p.measures.get(m)?.attributes.key_signature)
            {
                key = k.fifths.clamp(-7, 7);
            }
            let barline = shared_measure(score, m).map(|s| &s.barline);
            let ends = barline.is_some_and(|b| b.repeat_end.is_some());
            let next_starts = shared_measure(score, m + 1).is_some_and(|s| s.barline.repeat_start);
            let closing = match (ends, next_starts) {
                (true, true) => PriBarline::RepeatEndStart,
                (true, false) => PriBarline::RepeatEnd,
                (false, true) => PriBarline::RepeatStart,
                (false, false) => {
                    barline.map_or(PriBarline::Normal, |b| barline_kind_to_primus(b.kind))
                }
            };
            MeasureSpec::new(time, key).with_barline(closing)
        })
        .collect()
}

fn voice_events(part: &Part, m: usize, number: Option<u8>) -> &[Event] {
    let Some(measure) = part.measures.get(m) else {
        return &[];
    };
    match number {
        Some(n) => measure
            .voices
            .iter()
            .find(|v| v.number == n)
            .map_or(&[][..], |v| &v.events),
        None => &[],
    }
}

/// A measure's elements before it is fitted to its length, with the
/// hairpin mark each element carries.
struct BuiltMeasure {
    elements: Vec<(Element, Option<Wedge>)>,
}

impl BuiltMeasure {
    fn ticks(&self) -> u32 {
        self.elements.iter().filter_map(|(e, _)| e.ticks()).sum()
    }

    /// Puts a segno symbol on the first note or chord; `false` if there is
    /// none.
    fn add_segno(&mut self) -> bool {
        let segno = NoteSymbol::new(kind::SEGNO);
        for (element, _) in &mut self.elements {
            let symbols = match element {
                Element::Note(n) => &mut n.symbols,
                Element::Chord(c) => &mut c.symbols,
                _ => continue,
            };
            if symbols.len() < MAX_SYMBOLS {
                symbols.push(segno);
                return true;
            }
        }
        false
    }
}

fn is_grace(event: &Event) -> bool {
    match event {
        Event::Note(n) => n.grace,
        Event::Chord(notes) => notes.iter().all(|n| n.grace),
        Event::Rest(_) => false,
    }
}

fn build_measure(events: &[Event], percussion: bool) -> BuiltMeasure {
    let mut elements = Vec::new();
    for (i, event) in events.iter().enumerate() {
        if is_grace(event) {
            // A grace note needs a note or chord after it in its measure.
            let followed = events[i + 1..]
                .iter()
                .find(|e| !is_grace(e))
                .is_some_and(|e| !matches!(e, Event::Rest(_)));
            if percussion || !followed {
                continue;
            }
        }
        if let Some(element) = element(event, percussion) {
            let wedge = match event {
                Event::Note(n) => n.wedge,
                Event::Chord(notes) => notes.iter().find_map(|n| n.wedge),
                Event::Rest(_) => None,
            };
            elements.push((element, wedge));
        }
    }
    BuiltMeasure { elements }
}

/// A duration as a value PriMus writes; one it can't (more dots, an
/// uneven tuplet) as the nearest plain value.
fn value(duration: Duration) -> Option<Value> {
    value_from_duration(duration).or_else(|| {
        value_from_duration(Duration::from_quarter_length_quantized(
            duration.quarter_length(),
        ))
    })
}

fn element(event: &Event, percussion: bool) -> Option<Element> {
    let value = value(event.duration())?;
    let chord_symbol = event
        .chord_symbol()
        .and_then(|c| primus_scores::ChordSymbol::parse(&c.to_string()));
    let notes: &[Note] = match event {
        Event::Note(n) => std::slice::from_ref(n),
        Event::Chord(notes) => notes,
        Event::Rest(rest) => {
            let mut out = PriRest::new(value);
            if rest.fermata {
                out.symbols.push(NoteSymbol::new(kind::FERMATA));
            }
            out.chord_symbol = chord_symbol;
            return Some(if out.symbols.is_empty() && out.chord_symbol.is_none() {
                Element::Rest(value)
            } else {
                Element::RestWith(out)
            });
        }
    };
    let symbols = symbols(notes);
    let tremolo = notes
        .iter()
        .flat_map(|n| &n.ornaments)
        .find_map(|o| match o {
            Ornament::Tremolo(strokes) => Some(Tremolo::single((*strokes).clamp(1, 5))),
            _ => None,
        });

    if percussion {
        let mut sounds: Vec<u8> = notes
            .iter()
            .filter_map(|n| u8::try_from(n.pitch.to_midi_number()).ok())
            .filter(|&k| k <= 127)
            .collect();
        sounds.sort_unstable();
        sounds.dedup();
        if sounds.is_empty() {
            return Some(Element::Rest(value));
        }
        let mut drums = Drums::new(value, sounds);
        drums.symbols = symbols;
        return Some(Element::Drums(drums));
    }

    let mut tones: Vec<(u8, i8, bool)> = Vec::new();
    for n in notes {
        let Some((step, alteration)) = staff_step_from_pitch(n.pitch) else {
            continue;
        };
        if PriNote::new(step, alteration, value).midi_pitch().is_none()
            || tones.iter().any(|t| (t.0, t.1) == (step, alteration))
        {
            continue;
        }
        tones.push((
            step,
            alteration,
            matches!(n.tie, Tie::Start | Tie::StartStop),
        ));
    }
    if tones.is_empty() {
        return Some(Element::Rest(value));
    }
    if is_grace(event) {
        let pairs: Vec<(u8, i8)> = tones.iter().map(|t| (t.0, t.1)).collect();
        return Some(Element::Grace(Grace::chord(value, &pairs)));
    }
    let element = if let [(step, alteration, tied)] = tones[..] {
        let mut note = PriNote::new(step, alteration, value).with_symbols(symbols);
        note.tie_to_next = tied;
        note.chord_symbol = chord_symbol;
        note.tremolo = tremolo;
        Element::Note(note)
    } else {
        let pairs: Vec<(u8, i8)> = tones.iter().map(|t| (t.0, t.1)).collect();
        let mut chord = Chord::new(value, &pairs).with_symbols(symbols);
        for tone in &mut chord.tones {
            tone.tie_to_next = tones
                .iter()
                .any(|t| (t.0, t.1) == (tone.staff_step, tone.alteration) && t.2);
        }
        chord.chord_symbol = chord_symbol;
        chord.tremolo = tremolo;
        Element::Chord(chord)
    };
    Some(element)
}

/// A note's (or chord's) symbols, dynamic first (PriMus drops an
/// articulation listed before a dynamic).
fn symbols(notes: &[Note]) -> Vec<NoteSymbol> {
    let mut kinds: Vec<u8> = Vec::new();
    kinds.extend(
        notes
            .iter()
            .find_map(|n| n.dynamic.as_ref())
            .and_then(dynamic_kind),
    );
    for n in notes {
        kinds.extend(n.articulations.iter().flat_map(articulation_kinds));
        kinds.extend(n.ornaments.iter().filter_map(ornament_kind));
    }
    if notes.iter().any(|n| n.fermata) {
        kinds.push(kind::FERMATA);
    }
    let mut seen = BTreeSet::new();
    kinds.retain(|k| seen.insert(*k));
    kinds.truncate(MAX_SYMBOLS);
    kinds.into_iter().map(NoteSymbol::new).collect()
}

/// Each element's hairpin mark, for one measure.
type MeasureWedges = Vec<Option<Wedge>>;

/// A voice's measures fitted to `lengths` — gaps filled with rests, all-rest measures left empty (PriMus draws a
/// whole-measure rest) — and ties that don't reach a same-pitch next note
/// removed. Also each element's hairpin mark.
fn fit_voice(
    built: Vec<BuiltMeasure>,
    lengths: &[u32],
) -> Result<(Vec<PriMeasure>, Vec<MeasureWedges>)> {
    let mut measures = Vec::new();
    let mut wedges = Vec::new();
    for (m, (b, &length)) in built.into_iter().zip(lengths).enumerate() {
        let ticks = b.ticks();
        let (mut elements, mut marks): (Vec<_>, Vec<_>) = b.elements.into_iter().unzip();
        let rests = rests_filling(length - ticks).ok_or_else(|| {
            ScoresError::Write(format!(
                "measure {}: its notes can't be padded to the measure's length",
                m + 1
            ))
        })?;
        marks.extend(rests.iter().map(|_| None));
        elements.extend(rests.into_iter().map(Element::Rest));
        if elements.iter().all(|e| matches!(e, Element::Rest(_))) {
            elements.clear();
            marks.clear();
        }
        measures.push(PriMeasure {
            ticks: length,
            elements,
        });
        wedges.push(marks);
    }
    clear_dangling_ties(&mut measures);
    Ok((measures, wedges))
}

/// The pitches an element sounds, for matching ties.
fn tone_pitches(element: &Element) -> Vec<u8> {
    match element {
        Element::Note(n) => n.midi_pitch().into_iter().collect(),
        Element::Chord(c) => c.tones.iter().filter_map(|t| t.midi_pitch()).collect(),
        _ => Vec::new(),
    }
}

fn clear_dangling_ties(measures: &mut [PriMeasure]) {
    let positions: Vec<(usize, usize)> = measures
        .iter()
        .enumerate()
        .flat_map(|(m, measure)| (0..measure.elements.len()).map(move |e| (m, e)))
        .collect();
    for (i, &(m, e)) in positions.iter().enumerate() {
        let next = positions
            .get(i + 1)
            .map(|&(nm, ne)| tone_pitches(&measures[nm].elements[ne]))
            .unwrap_or_default();
        match &mut measures[m].elements[e] {
            Element::Note(n) => {
                if n.tie_to_next && !n.midi_pitch().is_some_and(|p| next.contains(&p)) {
                    n.tie_to_next = false;
                }
            }
            Element::Chord(c) => {
                for tone in &mut c.tones {
                    if tone.tie_to_next && !tone.midi_pitch().is_some_and(|p| next.contains(&p)) {
                        tone.tie_to_next = false;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Hairpins from their start and stop marks: each runs to the next stop,
/// or up to the next hairpin's start, or to the voice's last note.
fn wedge_spans(wedges: &[MeasureWedges], measures: &[PriMeasure]) -> Vec<Span> {
    let notes: Vec<(ElementRef, Option<Wedge>)> = measures
        .iter()
        .enumerate()
        .flat_map(|(m, measure)| {
            measure
                .elements
                .iter()
                .enumerate()
                .filter(|(_, e)| matches!(e, Element::Note(_) | Element::Chord(_)))
                .map(move |(e, _)| (ElementRef::new(m, e), wedges[m].get(e).copied().flatten()))
        })
        .collect();
    let mut spans = Vec::new();
    let mut open: Option<(ElementRef, Wedge)> = None;
    let close = |start: ElementRef, kind: Wedge, end: ElementRef| match kind {
        Wedge::CrescendoStart => Span::crescendo(start, end),
        _ => Span::decrescendo(start, end),
    };
    for (i, &(at, wedge)) in notes.iter().enumerate() {
        match wedge {
            Some(Wedge::Stop) => {
                if let Some((start, kind)) = open.take() {
                    spans.push(close(start, kind, at));
                }
            }
            Some(kind) => {
                if let Some((start, prev)) = open.take() {
                    let end = if i > 0 { notes[i - 1].0 } else { start };
                    spans.push(close(start, prev, end));
                }
                open = Some((at, kind));
            }
            None => {}
        }
    }
    if let (Some((start, kind)), Some(&(end, _))) = (open, notes.last()) {
        spans.push(close(start, kind, end));
    }
    spans
}
