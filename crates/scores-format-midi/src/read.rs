//! Converts a [`midly`] `Smf` into a [`Score`].
//!
//! MIDI has no notion of bar lines, ties or notated durations - only ticks -
//! so this reconstructs them heuristically: note on/off pairs are turned
//! into (possibly tied) runs of notated durations that add up to the exact
//! tick length, gaps between notes become rests, and measures are cut every
//! time the accumulated length reaches the time signature's length in
//! effect at that point. Tempo/time-signature `Meta` messages are read from
//! every track with their absolute tick position, and each part's own
//! measure-cutting walk (independent per part, so `Meta` messages found in
//! a non-conductor track are still honoured) records a change on whichever
//! measure its tick falls at the start of - mirroring how a notated score
//! only marks a change on the measure where it happens (a change undone
//! before the next measure starts isn't recorded). Simultaneous notes
//! with the same start/end tick become a chord (using the first note's
//! velocity for the whole chord). Tracks with no note events (e.g. a pure
//! conductor track) are not turned into parts. `SMPTE`-based timing is not
//! supported. Key signature is still read only once (the first occurrence
//! in any track, regardless of position) and applied to every part's first
//! measure, same as before.
//!
//! Note velocity is bucketed into a [`Dynamic`] marking, written on a note
//! only when it changes from the previous one (as a score would notate it),
//! via [`velocity_to_dynamic`]. A single note (or chord) whose velocity is
//! exactly the writer's accent/strong-accent boost above the surrounding
//! level, and which the next note doesn't stay at, is read as that
//! [`Articulation`](scores_core::Articulation) instead of a dynamic change (see [`crate::performance`]).
//!
//! What the writer stores as annotations is read back: a `Text` event whose
//! text is a chord symbol in its canonical form (`ChordSymbol::parse` then
//! `to_string` gives the same text - so e.g. lyrics or copyright text aren't
//! mistaken for chords) becomes the chord symbol of the event sounding at
//! that tick, and a fermata `Marker` sets `fermata` on the event starting
//! there - and undoes the writer's fermata tempo dip at that tick.
//!
//! Not read: multiple voices, ornaments, articulations other than accents
//! (a staccato note just reads as a shorter note plus a rest), grace notes
//! (indistinguishable from genuinely short notes), barline style, and
//! repeat/ending/navigation structure - the writer unrolls that, so a file
//! written by this plugin reads back as the unrolled measure sequence.

use crate::dynamics::velocity_to_dynamic;
use crate::performance::{FERMATA_MARKER, FERMATA_STRETCH, accent_from_boost};
use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use scores_core::{
    ChordSymbol, Clef, Duration, Dynamic, Event, KeySignature, Measure, MeasureAttributes, Mode,
    Note, NoteValue, Part, Pitch, Rest, Result, Score, ScoresError, Tempo, Tie, TimeSignature,
};
use std::collections::{HashMap, HashSet};

pub fn read(data: &[u8]) -> Result<Score> {
    let smf =
        Smf::parse(data).map_err(|e| ScoresError::Parse(format!("invalid MIDI file: {e}")))?;

    let ppq = match smf.header.timing {
        Timing::Metrical(ticks_per_beat) => u16::from(ticks_per_beat) as f64,
        Timing::Timecode(..) => {
            return Err(ScoresError::Unsupported(
                "SMPTE timecode-based MIDI timing is not supported".into(),
            ));
        }
    };

    let mut title: Option<String> = None;
    let mut key_signature: Option<KeySignature> = None;
    let mut raw_tempos: Vec<(u32, u32)> = Vec::new();
    let mut fermata_ticks: HashSet<u32> = HashSet::new();
    let mut time_sig_changes: Vec<(u32, TimeSignature)> = Vec::new();

    for track in &smf.tracks {
        let mut abs_tick: u32 = 0;
        for event in track {
            abs_tick += u32::from(event.delta);
            let TrackEventKind::Meta(meta) = event.kind else {
                continue;
            };
            match meta {
                MetaMessage::TrackName(name) if title.is_none() => {
                    title = decode_name(name);
                }
                MetaMessage::Tempo(micros_per_quarter) => {
                    raw_tempos.push((abs_tick, u32::from(micros_per_quarter)));
                }
                MetaMessage::Marker(text) if is_fermata_marker(text) => {
                    fermata_ticks.insert(abs_tick);
                }
                MetaMessage::TimeSignature(beats, beat_type_pow2, _, _) => {
                    time_sig_changes.push((
                        abs_tick,
                        TimeSignature {
                            beats,
                            beat_type: 1u8 << beat_type_pow2,
                        },
                    ));
                }
                MetaMessage::KeySignature(fifths, minor) if key_signature.is_none() => {
                    key_signature = Some(KeySignature {
                        fifths,
                        mode: if minor { Mode::Minor } else { Mode::Major },
                    });
                }
                _ => {}
            }
        }
    }

    // A tempo change at a fermata marker is the writer's fermata dip (or a
    // change inside one): dividing the micros back is exact.
    let tempo_changes: Vec<(u32, Tempo)> = raw_tempos
        .into_iter()
        .map(|(tick, micros)| {
            if fermata_ticks.contains(&tick) {
                (tick, micros / FERMATA_STRETCH)
            } else {
                (tick, micros)
            }
        })
        .filter(|&(_, micros)| micros > 0)
        .map(|(tick, micros)| {
            (
                tick,
                Tempo {
                    bpm: (60_000_000.0 / micros as f64) as f32,
                },
            )
        })
        .collect();

    let tempo_template = AttributeTimeline::new(tempo_changes, Tempo::default());
    let time_sig_template = AttributeTimeline::new(time_sig_changes, TimeSignature::default());

    let mut parts = Vec::new();
    for track in &smf.tracks {
        let mut track_name: Option<String> = None;
        let mut abs_tick: u32 = 0;
        let mut open: HashMap<u8, Vec<(u32, u8)>> = HashMap::new();
        let mut spans: Vec<(u32, u32, u8, u8)> = Vec::new();
        let mut annotations = Annotations::default();

        for event in track {
            abs_tick += u32::from(event.delta);
            match event.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(name)) => {
                    track_name = decode_name(name).or(track_name);
                }
                TrackEventKind::Meta(MetaMessage::Text(text)) => {
                    if let Some(symbol) = decode_chord_symbol(text) {
                        annotations.chord_symbols.push((abs_tick, symbol));
                    }
                }
                TrackEventKind::Meta(MetaMessage::Marker(text)) if is_fermata_marker(text) => {
                    annotations.fermatas.push(abs_tick);
                }
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { key, vel },
                    ..
                } => {
                    let key = u8::from(key);
                    let vel = u8::from(vel);
                    if vel > 0 {
                        open.entry(key).or_default().push((abs_tick, vel));
                    } else if let Some((start, velocity)) = open.get_mut(&key).and_then(Vec::pop) {
                        spans.push((start, abs_tick, key, velocity));
                    }
                }
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOff { key, .. },
                    ..
                } => {
                    let key = u8::from(key);
                    if let Some((start, velocity)) = open.get_mut(&key).and_then(Vec::pop) {
                        spans.push((start, abs_tick, key, velocity));
                    }
                }
                _ => {}
            }
        }

        if spans.is_empty() {
            continue;
        }

        spans.sort_by_key(|(start, _, key, _)| (*start, *key));
        annotations.chord_symbols.sort_by_key(|(tick, _)| *tick);
        annotations.fermatas.sort_unstable();
        let measures = build_measures(
            &spans,
            &annotations,
            ppq,
            time_sig_template.clone(),
            tempo_template.clone(),
        );

        parts.push(Part {
            id: format!("P{}", parts.len() + 1),
            name: track_name,
            measures,
        });
    }

    if parts.is_empty() {
        return Err(ScoresError::Parse(
            "MIDI file contains no note events".into(),
        ));
    }

    for part in &mut parts {
        if let Some(first) = part.measures.first_mut() {
            first.attributes.key_signature = Some(key_signature.unwrap_or_default());
            first.attributes.clef = Some(Clef::Treble);
        }
    }

    Ok(Score {
        title,
        composer: None,
        parts,
    })
}

fn decode_name(raw: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(raw).ok()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn is_fermata_marker(raw: &[u8]) -> bool {
    std::str::from_utf8(raw).is_ok_and(|text| text.trim().eq_ignore_ascii_case(FERMATA_MARKER))
}

/// A chord symbol, only if `raw` is exactly its canonical text form.
fn decode_chord_symbol(raw: &[u8]) -> Option<ChordSymbol> {
    let text = std::str::from_utf8(raw).ok()?.trim();
    ChordSymbol::parse(text).filter(|symbol| symbol.to_string() == text)
}

/// A track's chord symbol `Text` and fermata `Marker` events, sorted by tick.
#[derive(Default)]
struct Annotations {
    chord_symbols: Vec<(u32, ChordSymbol)>,
    fermatas: Vec<u32>,
}

/// A value (tempo or time signature) that changes at specific absolute tick
/// positions, advanced as a part's own tick cursor moves forward. Cloned
/// once per part from a shared template built from the whole file's `Meta`
/// messages, so each part's own measure-cutting walk sees the same global
/// changes but tracks "did this change since MY last measure" independently.
#[derive(Clone)]
struct AttributeTimeline<T> {
    changes: Vec<(u32, T)>,
    index: usize,
    current: T,
}

impl<T: Copy + PartialEq> AttributeTimeline<T> {
    fn new(mut changes: Vec<(u32, T)>, default: T) -> Self {
        changes.sort_by_key(|(tick, _)| *tick);
        let mut timeline = Self {
            changes,
            index: 0,
            current: default,
        };
        timeline.advance_to(0);
        timeline
    }

    /// Applies every not-yet-applied change at or before `tick`, returning
    /// whether the current value actually changed.
    fn advance_to(&mut self, tick: u32) -> bool {
        let before = self.current;
        while self.index < self.changes.len() && self.changes[self.index].0 <= tick {
            self.current = self.changes[self.index].1;
            self.index += 1;
        }
        self.current != before
    }
}

fn measure_quarter_length(ts: TimeSignature) -> f64 {
    ts.beats as f64 * 4.0 / ts.beat_type as f64
}

fn tick_threshold(ts: TimeSignature, ppq: f64) -> u32 {
    (measure_quarter_length(ts) * ppq).round().max(1.0) as u32
}

fn event_ticks(duration: Duration, ppq: f64) -> u32 {
    (duration.quarter_length() * ppq).round().max(1.0) as u32
}

/// Turns sorted `(start_tick, end_tick, key, velocity)` note spans for one
/// track into measures: gaps become rests, simultaneous same-start spans
/// become a chord, and any span too long to notate as a single [`Duration`]
/// becomes a tied run (see [`split_into_tied_durations`]). A measure is cut
/// every time the accumulated tick length reaches the current time
/// signature's length; `time_sig_tl`/`tempo_tl` are consulted at each cut
/// point (i.e. at the start of the new measure) to decide whether that
/// measure needs to record a change. `annotations` are attached to the
/// first produced event (note, chord or rest) still sounding at their tick.
fn build_measures(
    spans: &[(u32, u32, u8, u8)],
    annotations: &Annotations,
    ppq: f64,
    mut time_sig_tl: AttributeTimeline<TimeSignature>,
    mut tempo_tl: AttributeTimeline<Tempo>,
) -> Vec<Measure> {
    let mut measures: Vec<Measure> = Vec::new();
    let mut current_events: Vec<Event> = Vec::new();
    let mut cursor: u32 = 0;
    let mut measure_start_tick: u32 = 0;
    let mut accumulated_ticks: u32 = 0;
    let mut measure_number: u32 = 1;
    let mut last_dynamic: Option<Dynamic> = None;
    let mut base_velocity: Option<u8> = None;
    let mut next_chord_symbol = 0;
    let mut next_fermata = 0;

    let mut current_ts = time_sig_tl.current;
    let mut threshold = tick_threshold(current_ts, ppq);
    // Measure 1 always records the starting time signature/tempo, matching
    // the model's own "first measure establishes the initial value"
    // convention (see `scores_core::Measure`'s docs).
    let mut pending_attributes = Some(MeasureAttributes {
        time_signature: Some(current_ts),
        tempo: Some(tempo_tl.current),
        key_signature: None,
        clef: None,
    });

    let mut i = 0;
    while i < spans.len() {
        let start = spans[i].0;
        let mut end = spans[i].1;
        let mut keys = Vec::new();
        let velocity = spans[i].3;
        let mut j = i;
        while j < spans.len() && spans[j].0 == start {
            keys.push(spans[j].2);
            end = end.max(spans[j].1);
            j += 1;
        }

        // (event, notated ticks, start tick)
        let mut produced: Vec<(Event, u32, u32)> = Vec::new();

        if start > cursor {
            let gap_quarters = (start - cursor) as f64 / ppq;
            let mut rest_start = cursor;
            for duration in split_into_tied_durations(gap_quarters) {
                let ticks = event_ticks(duration, ppq);
                produced.push((Event::Rest(Rest::new(duration)), ticks, rest_start));
                rest_start += ticks;
            }
        }

        // An isolated velocity rise of exactly an accent boost is an accent
        // on top of the running level, not a new dynamic.
        let next_velocity = spans.get(j).map(|span| span.3);
        let accent = base_velocity
            .or(next_velocity)
            .and_then(|base| accent_from_boost(velocity as i16 - base as i16))
            .filter(|_| next_velocity != Some(velocity));
        let level = match (&accent, base_velocity.or(next_velocity)) {
            (Some(_), Some(base)) => base,
            _ => {
                base_velocity = Some(velocity);
                velocity
            }
        };
        let mut piece_start = start;

        let span_quarters = (end - start) as f64 / ppq;
        let pieces = split_into_tied_durations(span_quarters);
        let piece_count = pieces.len();
        for (position, duration) in pieces.into_iter().enumerate() {
            let tie = tie_for_position(position, piece_count);
            // Only the first piece of a tied run carries the dynamic - and
            // only when it actually changed, same as a notated score would.
            let dynamic = if position == 0 {
                let candidate = velocity_to_dynamic(level);
                (last_dynamic.as_ref() != Some(&candidate)).then(|| {
                    last_dynamic = Some(candidate.clone());
                    candidate
                })
            } else {
                None
            };
            let articulations: Vec<_> = accent.iter().filter(|_| position == 0).cloned().collect();
            let ticks = event_ticks(duration, ppq);

            if keys.len() == 1 {
                let mut note = Note::new(
                    Pitch::from_midi_number(keys[0] as i32, false),
                    duration,
                    tie,
                );
                note.dynamic = dynamic;
                note.articulations = articulations;
                produced.push((Event::Note(note), ticks, piece_start));
            } else {
                let mut notes: Vec<Note> = keys
                    .iter()
                    .map(|&key| {
                        Note::new(Pitch::from_midi_number(key as i32, false), duration, tie)
                    })
                    .collect();
                if let Some(first) = notes.first_mut() {
                    first.dynamic = dynamic;
                    first.articulations = articulations;
                }
                produced.push((Event::Chord(notes), ticks, piece_start));
            }
            piece_start += ticks;
        }

        for (mut event, ticks, event_start) in produced {
            let event_end = event_start + ticks;
            while let Some((tick, symbol)) = annotations.chord_symbols.get(next_chord_symbol) {
                if *tick >= event_end {
                    break;
                }
                if event.chord_symbol().is_none() {
                    event.set_chord_symbol(Some(symbol.clone()));
                }
                next_chord_symbol += 1;
            }
            while annotations
                .fermatas
                .get(next_fermata)
                .is_some_and(|&tick| tick < event_end)
            {
                set_fermata(&mut event);
                next_fermata += 1;
            }

            current_events.push(event);
            accumulated_ticks += ticks;
            if accumulated_ticks >= threshold {
                let attributes = pending_attributes.take().unwrap_or_default();
                measures.push(Measure::single_voice(
                    measure_number,
                    attributes,
                    std::mem::take(&mut current_events),
                ));
                measure_number += 1;
                measure_start_tick += accumulated_ticks;
                accumulated_ticks = 0;

                let ts_changed = time_sig_tl.advance_to(measure_start_tick);
                let tempo_changed = tempo_tl.advance_to(measure_start_tick);
                if ts_changed {
                    current_ts = time_sig_tl.current;
                    threshold = tick_threshold(current_ts, ppq);
                }
                if ts_changed || tempo_changed {
                    pending_attributes = Some(MeasureAttributes {
                        time_signature: ts_changed.then_some(current_ts),
                        tempo: tempo_changed.then_some(tempo_tl.current),
                        key_signature: None,
                        clef: None,
                    });
                }
            }
        }

        cursor = end;
        i = j;
    }

    if !current_events.is_empty() || measures.is_empty() {
        let attributes = pending_attributes.unwrap_or_default();
        measures.push(Measure::single_voice(
            measure_number,
            attributes,
            current_events,
        ));
    }

    measures
}

fn set_fermata(event: &mut Event) {
    match event {
        Event::Note(note) => note.fermata = true,
        Event::Chord(notes) => notes.iter_mut().for_each(|n| n.fermata = true),
        Event::Rest(rest) => rest.fermata = true,
    }
}

fn tie_for_position(position: usize, total: usize) -> Tie {
    if total <= 1 {
        Tie::None
    } else if position == 0 {
        Tie::Start
    } else if position == total - 1 {
        Tie::Stop
    } else {
        Tie::StartStop
    }
}

/// Decomposes an arbitrary length (in quarter notes) into the fewest
/// plain (non-tuplet), up-to-double-dotted durations that sum to it,
/// greedily taking the largest one that still fits at each step.
fn split_into_tied_durations(mut remaining: f64) -> Vec<Duration> {
    const EPSILON: f64 = 1e-2;
    let mut parts = Vec::new();
    while remaining > EPSILON {
        let duration = largest_duration_leq(remaining);
        remaining -= duration.quarter_length();
        parts.push(duration);
    }
    if parts.is_empty() {
        parts.push(Duration::new(NoteValue::SixtyFourth));
    }
    parts
}

fn largest_duration_leq(target: f64) -> Duration {
    let mut best = Duration::new(NoteValue::SixtyFourth);
    let mut best_length = best.quarter_length();
    for base in NoteValue::ALL {
        for dots in 0..=2u8 {
            let candidate = Duration::dotted(base, dots);
            let length = candidate.quarter_length();
            if length <= target + 1e-9 && length > best_length {
                best = candidate;
                best_length = length;
            }
        }
    }
    best
}
