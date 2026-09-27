use std::collections::HashMap;

use primus_scores::measures::Barline as PriBarline;
use primus_scores::{DecodedNote, HairpinDirection, PriDocument, PriFile, SpanEntry};
use scores_core::{
    Barline, ChordSymbol, Duration, Ending, Event, KeySignature, Measure, MeasureAttributes, Mode,
    Navigation, Note, Ornament, Part, Rest, Result, Score, ScoresError, Tempo, Tie, TimeSignature,
    Voice, Wedge,
};

use crate::convert::{
    barline_kind_from_primus, clef_from_primus, duration_from_ticks, ending_numbers, is_fermata,
    marks_from_symbols, navigation_from_text, pitch_from_staff_step, rest_durations_filling,
};

pub fn read(data: &[u8]) -> Result<Score> {
    let file = PriFile::from_bytes(data)
        .map_err(|e| ScoresError::Parse(format!("not a .pri file: {e}")))?;
    let doc = &file.document;
    let measure_count = doc
        .measure_count()
        .ok_or_else(|| ScoresError::Parse("no readable measure table".into()))?;
    let measure_ticks = doc.measure_ticks().unwrap_or_default();

    let header = doc.header_texts().unwrap_or_default();
    let shared = shared_measures(doc, measure_count);

    let staff_count = doc
        .piece_instruments(0)
        .map_or(doc.instrument_count(), |r| r.end);
    let mut staff_voices: Vec<Vec<usize>> = vec![Vec::new(); staff_count];
    // A later piece's voices start over at staff 0; only the first piece is
    // read.
    let mut last_staff = 0;
    for v in 0..doc.voice_count() {
        let Some(staff) = doc.voice_instrument(v) else {
            continue;
        };
        if staff < last_staff || staff >= staff_count {
            break;
        }
        last_staff = staff;
        staff_voices[staff].push(v);
    }

    let mut parts = Vec::new();
    for (staff, voices) in staff_voices.iter().enumerate() {
        if voices.is_empty() {
            continue;
        }
        let mut measures: Vec<Measure> = shared
            .iter()
            .enumerate()
            .map(|(m, s)| Measure {
                number: m as u32 + 1,
                attributes: s.attributes.clone(),
                voices: Vec::new(),
                barline: s.barline.clone(),
                navigation: s.navigation.clone(),
            })
            .collect();
        if let Some(first) = measures.first_mut() {
            first.attributes.clef = doc.instrument_clef(staff).map(clef_from_primus);
        }
        for change in doc.instrument_clef_changes(staff).unwrap_or_default() {
            if let Some(m) = measures.get_mut(change.measure as usize) {
                m.attributes.clef = Some(clef_from_primus(change.clef));
            }
        }

        let mut number = 0u8;
        for (k, &v) in voices.iter().enumerate() {
            let Some(by_measure) = read_voice(doc, v, measure_count) else {
                continue;
            };
            let has_content = by_measure.events.iter().any(|e| !e.is_empty());
            // A staff's further voices are only kept where they have notes.
            if k > 0 && !has_content {
                continue;
            }
            number += 1;
            for (m, events) in by_measure.events.into_iter().enumerate() {
                let events = if events.is_empty() && k == 0 {
                    let ticks = measure_ticks.get(m).copied().unwrap_or(0);
                    rest_durations_filling(ticks)
                        .into_iter()
                        .map(|d| Event::Rest(Rest::new(d)))
                        .collect()
                } else {
                    events
                };
                measures[m].voices.push(Voice { number, events });
            }
            for m in by_measure.segno {
                if !measures[m].navigation.contains(&Navigation::Segno) {
                    measures[m].navigation.insert(0, Navigation::Segno);
                }
            }
        }
        let name = doc
            .instrument_names(staff)
            .map(|n| n.long)
            .filter(|n| !n.trim().is_empty());
        parts.push(Part {
            id: format!("P{}", parts.len() + 1),
            name,
            measures,
        });
    }

    Ok(Score {
        title: header
            .title
            .or_else(|| doc.title())
            .filter(|t| !t.is_empty()),
        composer: header.composer.filter(|t| !t.is_empty()),
        parts,
    })
}

/// What every staff of a measure shares: its signatures and tempo, its
/// barline, repeats and endings, and navigation marks.
struct Shared {
    attributes: MeasureAttributes,
    barline: Barline,
    navigation: Vec<Navigation>,
}

fn shared_measures(doc: &PriDocument, count: usize) -> Vec<Shared> {
    let times = doc.time_signatures().unwrap_or_default();
    let keys = doc.key_signatures().unwrap_or_default();
    let barlines = doc.barlines().unwrap_or_default();
    let mut out: Vec<Shared> = (0..count)
        .map(|_| Shared {
            attributes: MeasureAttributes::default(),
            barline: Barline::default(),
            navigation: Vec::new(),
        })
        .collect();

    let (mut last_time, mut last_key) = (None, None);
    for (m, shared) in out.iter_mut().enumerate() {
        if let Some(t) = times.get(m) {
            let time = TimeSignature {
                beats: t.numerator,
                beat_type: t.denominator,
            };
            if last_time != Some(time) {
                shared.attributes.time_signature = Some(time);
                last_time = Some(time);
            }
        }
        if let Some(&fifths) = keys.get(m)
            && last_key != Some(fifths)
        {
            shared.attributes.key_signature = Some(KeySignature {
                fifths,
                mode: Mode::Major,
            });
            last_key = Some(fifths);
        }
    }
    if let Some(first) = out.first_mut() {
        first.attributes.tempo = doc.piece_tempo(0).map(|t| Tempo {
            bpm: t.bpm() as f32,
        });
    }
    for mark in doc.piece_tempo_marks(0).unwrap_or_default() {
        if let (Some(shared), Some(tempo)) = (out.get_mut(mark.measure), mark.tempo()) {
            shared.attributes.tempo = Some(Tempo {
                bpm: tempo.bpm() as f32,
            });
        }
    }

    for (m, &barline) in barlines.iter().enumerate().take(count) {
        out[m].barline.kind = barline_kind_from_primus(barline);
        let (ends, starts) = match barline {
            PriBarline::RepeatEnd => (true, false),
            PriBarline::RepeatStart => (false, true),
            PriBarline::RepeatEndStart | PriBarline::RepeatEndStartHeavy => (true, true),
            _ => (false, false),
        };
        if ends {
            out[m].barline.repeat_end = Some(2);
        }
        if starts && let Some(next) = out.get_mut(m + 1) {
            next.barline.repeat_start = true;
        }
    }
    for ending in doc.repeat_endings().unwrap_or_default() {
        let numbers = ending_numbers(&ending.text);
        let first = usize::from(ending.first);
        if numbers.is_empty() || ending.last < ending.first || first >= count {
            continue;
        }
        out[first].barline.ending = Some(Ending {
            numbers,
            measures: u32::from(ending.last - ending.first) + 1,
            closed: !ending.open,
        });
    }
    for text in doc.measure_texts().unwrap_or_default() {
        if let (Some(shared), Some(nav)) =
            (out.get_mut(text.measure), navigation_from_text(&text.text))
            && !shared.navigation.contains(&nav)
        {
            shared.navigation.push(nav);
        }
    }
    out
}

/// One voice's events, per measure, and the measures a segno symbol is in.
struct VoiceContent {
    events: Vec<Vec<Event>>,
    segno: Vec<usize>,
}

/// An element of a voice's `ELST`: a note or chord (a range of
/// `voice_notes`), or a rest (an index into `voice_rests`).
enum Record {
    Notes(std::ops::Range<usize>),
    Rest(usize),
}

fn read_voice(doc: &PriDocument, v: usize, measure_count: usize) -> Option<VoiceContent> {
    let notes = doc.voice_notes(v)?;
    let rests = doc.voice_rests(v).unwrap_or_default();

    // Chord tones are consecutive and share their record's identity.
    let mut records: Vec<(usize, Option<usize>, Record)> = Vec::new();
    let mut i = 0;
    while i < notes.len() {
        let mut j = i + 1;
        while j < notes.len()
            && notes[i].identity.is_some()
            && notes[j].identity == notes[i].identity
        {
            j += 1;
        }
        records.push((notes[i].offset, notes[i].measure, Record::Notes(i..j)));
        i = j;
    }
    for (r, rest) in rests.iter().enumerate() {
        records.push((rest.offset, rest.measure, Record::Rest(r)));
    }
    records.sort_by_key(|(offset, ..)| *offset);

    let symbols: HashMap<usize, _> = doc
        .voice_note_symbols(v)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let chords: HashMap<usize, _> = doc
        .voice_chord_symbols(v)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let tremolos: HashMap<usize, _> = doc
        .voice_tremolos(v)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let graces: HashMap<usize, _> = doc
        .voice_grace_notes(v)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let rest_symbols: HashMap<usize, _> = doc
        .voice_rest_symbols(v)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let rest_chords: HashMap<usize, _> = doc
        .voice_rest_chord_symbols(v)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let ties = tie_flags(doc, v, &notes);
    let wedges = wedges(doc, v, &notes);

    let mut content = VoiceContent {
        events: vec![Vec::new(); measure_count],
        segno: Vec::new(),
    };
    let mut measure = 0;
    for (_, record_measure, record) in records {
        if let Some(m) = record_measure {
            measure = m;
        }
        if measure >= measure_count {
            continue;
        }
        let event = match record {
            Record::Rest(r) => {
                let rest = &rests[r];
                let ticks = rest
                    .duration_ticks
                    .or(rest.duration_sixteenths.map(|s| u32::from(s) * 240));
                let mut out = Rest::new(
                    ticks.map_or_else(Duration::default, |t| duration_from_ticks(t, None)),
                );
                out.fermata =
                    rest_symbols
                        .get(&r)
                        .is_some_and(|s: &Vec<primus_scores::NoteSymbol>| {
                            s.iter().any(|s| is_fermata(s.kind))
                        });
                out.chord_symbol = rest_chords.get(&r).and_then(chord_symbol);
                Event::Rest(out)
            }
            Record::Notes(range) => {
                let first = &notes[range.start];
                let ticks = first
                    .duration_ticks
                    .or(first.duration_sixteenths.map(|s| u32::from(s) * 240));
                let duration = ticks.map_or_else(Duration::default, |t| {
                    duration_from_ticks(t, first.tuplet_marker)
                });
                let grace = graces.contains_key(&range.start);
                let mut tones: Vec<Note> = range
                    .clone()
                    .map(|n| {
                        let mut note = Note::new(pitch(&notes[n]), duration, ties[n]);
                        note.grace = grace;
                        note
                    })
                    .collect();
                let head = &mut tones[0];
                if let Some(symbols) = symbols.get(&range.start) {
                    let marks = marks_from_symbols(symbols);
                    head.dynamic = marks.dynamic;
                    head.articulations = marks.articulations;
                    head.ornaments = marks.ornaments;
                    head.fermata = marks.fermata;
                    if marks.segno {
                        content.segno.push(measure);
                    }
                }
                if let Some(tremolo) = tremolos.get(&range.start).filter(|t| !t.alternating) {
                    head.ornaments.push(Ornament::Tremolo(tremolo.strokes));
                }
                head.chord_symbol = chords.get(&range.start).and_then(chord_symbol);
                head.wedge = wedges.get(&range.start).copied();
                if tones.len() == 1 {
                    Event::Note(tones.pop().unwrap())
                } else {
                    Event::Chord(tones)
                }
            }
        };
        content.events[measure].push(event);
    }
    Some(content)
}

fn pitch(note: &DecodedNote) -> scores_core::Pitch {
    match (note.staff_step, note.alteration) {
        (Some(step), Some(alteration)) if !note.percussion => {
            pitch_from_staff_step(step, alteration)
        }
        _ => scores_core::Pitch::from_midi_number(i32::from(note.pitch), false),
    }
}

fn chord_symbol(chord: &primus_scores::ChordSymbol) -> Option<ChordSymbol> {
    ChordSymbol::parse(&chord.name())
}

/// Each note's tie, from the voice's `TIES` chunk (what PriMus draws),
/// or else from the notes' own tie flags.
fn tie_flags(doc: &PriDocument, v: usize, notes: &[DecodedNote]) -> Vec<Tie> {
    let mut starts = vec![false; notes.len()];
    let mut stops = vec![false; notes.len()];
    match doc.voice_ties(v) {
        Some(ties) => {
            for tie in ties {
                let (Some(start), Some(end)) = (tie.start_note, tie.end_note) else {
                    continue;
                };
                let tones: Vec<(usize, usize)> = if tie.tones.is_empty() {
                    vec![(0, 0)]
                } else {
                    tie.tones
                        .iter()
                        .map(|t| (usize::from(t.start_tone), usize::from(t.end_tone)))
                        .collect()
                };
                for (s, e) in tones {
                    let (s, e) = (start + s, end + e);
                    if s < notes.len() && e < notes.len() {
                        starts[s] = true;
                        stops[e] = true;
                    }
                }
            }
        }
        None => {
            for (i, note) in notes.iter().enumerate() {
                if note.tied_to_next == Some(true)
                    && let Some(j) = (i + 1..notes.len()).find(|&j| {
                        notes[j].pitch == note.pitch && notes[j].identity != note.identity
                    })
                {
                    starts[i] = true;
                    stops[j] = true;
                }
            }
        }
    }
    starts
        .into_iter()
        .zip(stops)
        .map(|(start, stop)| match (start, stop) {
            (true, true) => Tie::StartStop,
            (true, false) => Tie::Start,
            (false, true) => Tie::Stop,
            (false, false) => Tie::None,
        })
        .collect()
}

/// Hairpin starts and ends, by the index of their note's first tone.
fn wedges(doc: &PriDocument, v: usize, notes: &[DecodedNote]) -> HashMap<usize, Wedge> {
    let by_identity: HashMap<u32, usize> = notes
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(i, n)| Some((n.identity?, i)))
        .collect();
    let mut out = HashMap::new();
    for entry in doc.voice_span_entries(v).unwrap_or_default() {
        let SpanEntry::Hairpin(hairpin) = entry else {
            continue;
        };
        let start = match hairpin.direction {
            HairpinDirection::Crescendo => Wedge::CrescendoStart,
            HairpinDirection::Decrescendo => Wedge::DiminuendoStart,
            HairpinDirection::Unknown(_) => continue,
        };
        let (Some(&s), Some(&e)) = (
            by_identity.get(&hairpin.start_note_ref),
            by_identity.get(&hairpin.end_note_ref),
        ) else {
            continue;
        };
        out.insert(s, start);
        if e != s {
            out.entry(e).or_insert(Wedge::Stop);
        }
    }
    out
}
