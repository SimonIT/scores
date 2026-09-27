//! Builds a [`midly`] `Smf` from a [`Score`] and serializes it with
//! `Smf::write_std`, rather than emitting MIDI bytes by hand.
//!
//! A MIDI file is a performance, so the score is first *unrolled*: measures
//! are written in playback order, taking repeats, 1st/2nd endings and
//! D.C./D.S./Coda/Fine jumps (see [`scores_core::playback_order`]). That
//! order is computed from the first part and applied to every part so they
//! stay aligned. The
//! structure itself (barlines, endings, navigation marks) is realized rather
//! than written, so nothing re-reads it as structure - reading the file back
//! yields the unrolled measures.
//!
//! Track 0 is a conductor track carrying the title, key signature (from the
//! first part's first measure) and every tempo/time-signature change: it
//! walks the first part's measures in playback order, emitting a `Meta`
//! message wherever the tempo/time signature in effect changes (so jumping
//! back into an earlier section restores that section's values), falling
//! back to a default at tick 0. Fermatas are rendered as tempo dips over the
//! fermata event (see [`crate::performance`]). One further track is written
//! per [`Part`], using only its first voice (MIDI has no multi-voice concept
//! to write a second one into).
//!
//! In a part track, chords are written without tie merging (each note gets
//! its own on/off pair); single notes honour [`Tie`] by holding one sounding
//! note across tied events. A note's [`Dynamic`]
//! becomes its on-velocity (see [`crate::dynamics`]) and persists until
//! changed, the way a notated dynamic would; a crescendo/diminuendo
//! [`Wedge`] ramps the velocity linearly (by tick) from its start note to
//! its stop note, towards the dynamic written on (or right after) the stop
//! note, or one dynamic step ([`WEDGE_STEP`]) if there is none. Articulations,
//! ornaments and grace notes change only sounding length/onset/velocity (see
//! [`crate::performance`]). Chord symbols become `Text` meta events (in
//! their canonical [`ChordSymbol`](scores_core::ChordSymbol) text form) at
//! the event's tick and fermatas a `Marker`, both in the part's own track.
//! Barline style has no MIDI equivalent and is not written.

use crate::dynamics::{DEFAULT_VELOCITY, dynamic_to_velocity};
use crate::performance::{
    FERMATA_MARKER, FERMATA_STRETCH, PPQ, accented_velocity, clamp_key, grace_length,
    realize_ornament, sounding_ticks,
};
use midly::num::{u4, u7, u15, u24, u28};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};
use scores_core::{
    Articulation, Duration, Dynamic, Event, KeySignature, Measure, Mode, Note, Part, Result, Score,
    ScoresError, Tempo, Tie, TimeSignature, Wedge, playback_order,
};

/// Velocity change of a hairpin with no target dynamic (one dynamic level).
const WEDGE_STEP: i32 = 16;

pub fn write(score: &Score) -> Result<Vec<u8>> {
    let header = Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQ)));

    let order = score
        .parts
        .first()
        .map(|part| playback_order(&part.measures))
        .unwrap_or_default();
    let timelines: Vec<Timeline<'_>> = score
        .parts
        .iter()
        .map(|part| Timeline::new(part, &order))
        .collect();
    let fermatas = fermata_spans(&timelines);

    let mut tracks: Vec<Track> = Vec::with_capacity(score.parts.len() + 1);
    tracks.push(conductor_track(score, &order, &fermatas));
    for (index, (part, timeline)) in score.parts.iter().zip(&timelines).enumerate() {
        tracks.push(part_track(part, timeline, (index % 16) as u8));
    }

    let smf = Smf { header, tracks };
    let mut buf = Vec::new();
    smf.write_std(&mut buf)
        .map_err(|e| ScoresError::Write(format!("failed to serialize MIDI file: {e}")))?;
    Ok(buf)
}

/// One event of a part's first voice at its absolute tick in the unrolled
/// timeline.
struct Placed<'a> {
    tick: u32,
    /// Notated length in ticks (0 for a grace note).
    ticks: u32,
    event: &'a Event,
    key_signature: KeySignature,
    chord_text: Option<String>,
}

/// A part's first voice flattened into playback order. Owns the chord
/// symbol texts so the track's `Text` events can borrow them.
struct Timeline<'a> {
    events: Vec<Placed<'a>>,
}

impl<'a> Timeline<'a> {
    fn new(part: &'a Part, order: &[usize]) -> Self {
        let key_signatures = effective(&part.measures, |m| m.attributes.key_signature);
        let mut events = Vec::new();
        let mut tick = 0;
        for &index in order {
            let Some(voice) = part.measures.get(index).and_then(|m| m.voices.first()) else {
                continue;
            };
            for event in &voice.events {
                let ticks = event_ticks(event);
                events.push(Placed {
                    tick,
                    ticks,
                    event,
                    key_signature: key_signatures[index],
                    chord_text: event.chord_symbol().map(ToString::to_string),
                });
                tick += ticks;
            }
        }
        Self { events }
    }
}

/// The value of a per-measure attribute in effect at each measure (in
/// written order), since the model only records it where it changes.
fn effective<T: Copy + Default>(
    measures: &[Measure],
    get: impl Fn(&Measure) -> Option<T>,
) -> Vec<T> {
    let mut current = T::default();
    measures
        .iter()
        .map(|measure| {
            if let Some(value) = get(measure) {
                current = value;
            }
            current
        })
        .collect()
}

/// The merged `[start, end)` tick spans of every fermata event in any part.
fn fermata_spans(timelines: &[Timeline<'_>]) -> Vec<(u32, u32)> {
    let mut spans: Vec<(u32, u32)> = timelines
        .iter()
        .flat_map(|timeline| &timeline.events)
        .filter(|placed| placed.ticks > 0 && has_fermata(placed.event))
        .map(|placed| (placed.tick, placed.tick + placed.ticks))
        .collect();
    spans.sort();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

fn has_fermata(event: &Event) -> bool {
    match event {
        Event::Note(note) => note.fermata,
        Event::Chord(notes) => notes.iter().any(|n| n.fermata),
        Event::Rest(rest) => rest.fermata,
    }
}

/// Walks the first part's measures in playback order to place a `Meta`
/// message at the exact tick where the tempo/time signature in effect
/// changes, using the same per-measure tick length ([`measure_ticks`]) the
/// note data itself advances by. Fermata spans are overlaid on the tempo
/// (see [`tempo_events`]).
fn conductor_track<'a>(score: &'a Score, order: &[usize], fermatas: &[(u32, u32)]) -> Track<'a> {
    let key_signature = score
        .parts
        .first()
        .and_then(|p| p.measures.first())
        .and_then(|m| m.attributes.key_signature)
        .unwrap_or_default();

    let mut events: Vec<(u32, TrackEventKind<'_>)> = Vec::new();
    if let Some(title) = &score.title {
        events.push((
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(title.as_bytes())),
        ));
    }

    let mut base_tempos: Vec<(u32, u32)> = Vec::new();
    let mut tick: u32 = 0;
    let mut last_time_signature = None;
    if let Some(part) = score.parts.first() {
        let tempos = effective(&part.measures, |m| m.attributes.tempo);
        let time_signatures = effective(&part.measures, |m| m.attributes.time_signature);
        for &index in order {
            let Some(measure) = part.measures.get(index) else {
                continue;
            };
            let micros = tempo_micros(tempos[index]);
            if base_tempos.last().map(|&(_, m)| m) != Some(micros) {
                base_tempos.push((tick, micros));
            }
            if last_time_signature != Some(time_signatures[index]) {
                events.push((tick, time_signature_event(time_signatures[index])));
                last_time_signature = Some(time_signatures[index]);
            }
            tick += measure_ticks(measure);
        }
    }
    if base_tempos.is_empty() {
        base_tempos.push((0, tempo_micros(Tempo::default())));
    }
    if last_time_signature.is_none() {
        events.push((0, time_signature_event(TimeSignature::default())));
    }
    for (event_tick, micros) in tempo_events(&base_tempos, fermatas) {
        events.push((
            event_tick,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(micros))),
        ));
    }
    events.push((
        0,
        TrackEventKind::Meta(MetaMessage::KeySignature(
            key_signature.fifths,
            key_signature.mode == Mode::Minor,
        )),
    ));

    events.sort_by_key(|(event_tick, _)| *event_tick);
    let end = tick.max(fermatas.last().map_or(0, |span| span.1));
    to_track(events, end)
}

/// Overlays fermata spans on the base tempo map (`(tick, micros per
/// quarter)`): inside a span the tempo is [`FERMATA_STRETCH`] times slower.
/// An event is always written at each span's start and end - even if the
/// value happens not to change there - since the reader identifies the dip
/// by the fermata marker at its start tick and undoes it exactly by dividing
/// the micros back.
fn tempo_events(base: &[(u32, u32)], fermatas: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let base_at = |tick: u32| {
        base.iter()
            .rev()
            .find(|(start, _)| *start <= tick)
            .map_or(base[0].1, |&(_, micros)| micros)
    };
    let mut boundaries: Vec<(u32, bool)> = base.iter().map(|&(tick, _)| (tick, false)).collect();
    for &(start, end) in fermatas {
        boundaries.push((start, true));
        boundaries.push((end, true));
    }
    boundaries.sort();

    let mut out: Vec<(u32, u32)> = Vec::new();
    for (index, &(tick, forced)) in boundaries.iter().enumerate() {
        // Sorted, so a forced duplicate of a tick comes last - only emit once.
        if boundaries.get(index + 1).is_some_and(|next| next.0 == tick) {
            continue;
        }
        let inside = fermatas.iter().any(|&(s, e)| s <= tick && tick < e);
        let micros = if inside {
            (base_at(tick) * FERMATA_STRETCH).min(0xFF_FFFF)
        } else {
            base_at(tick)
        };
        if forced || out.last().map(|&(_, m)| m) != Some(micros) {
            out.push((tick, micros));
        }
    }
    out
}

fn tempo_micros(tempo: Tempo) -> u32 {
    (60_000_000.0 / tempo.bpm as f64)
        .round()
        .clamp(1.0, 0xFF_FFFF as f64) as u32
}

fn time_signature_event<'a>(ts: TimeSignature) -> TrackEventKind<'a> {
    let denominator_pow2 = (ts.beat_type as f64).log2().round() as u8;
    TrackEventKind::Meta(MetaMessage::TimeSignature(
        ts.beats,
        denominator_pow2,
        24,
        8,
    ))
}

/// The tick length of one measure's first voice - the same quantity a part
/// timeline advances by - used to place conductor `Meta` messages at the
/// tick where the corresponding measure begins.
fn measure_ticks(measure: &Measure) -> u32 {
    measure
        .voices
        .first()
        .map(|voice| voice.events.iter().map(event_ticks).sum())
        .unwrap_or(0)
}

/// Notated length in ticks; a grace note takes none.
fn event_ticks(event: &Event) -> u32 {
    match event {
        Event::Rest(rest) => duration_ticks(&rest.duration),
        Event::Note(note) => note_ticks(note),
        Event::Chord(notes) => notes.iter().map(note_ticks).max().unwrap_or(0),
    }
}

fn is_grace(event: &Event) -> bool {
    match event {
        Event::Note(note) => note.grace,
        Event::Chord(notes) => notes.first().is_some_and(|n| n.grace),
        Event::Rest(_) => false,
    }
}

fn event_notes(event: &Event) -> &[Note] {
    match event {
        Event::Note(note) => std::slice::from_ref(note),
        Event::Chord(notes) => notes,
        Event::Rest(_) => &[],
    }
}

fn event_dynamic(event: &Event) -> Option<&Dynamic> {
    event_notes(event).iter().find_map(|n| n.dynamic.as_ref())
}

fn event_wedge(event: &Event) -> Option<Wedge> {
    event_notes(event).iter().find_map(|n| n.wedge)
}

/// Articulations of an event (for a chord, of all its notes).
fn event_articulations(event: &Event) -> Vec<&Articulation> {
    event_notes(event)
        .iter()
        .flat_map(|n| &n.articulations)
        .collect()
}

/// The dynamic-level velocity of every event (before accents): explicit
/// dynamics persist until changed, and a wedge ramps between its start and
/// stop events (see the module docs). An explicit dynamic inside a wedge
/// takes over from the ramp.
fn base_velocities(events: &[Placed<'_>]) -> Vec<u8> {
    struct Ramp {
        from: (u32, i32),
        to: (u32, i32),
        stop: usize,
    }

    let mut velocities = Vec::with_capacity(events.len());
    let mut current = DEFAULT_VELOCITY as i32;
    let mut ramp: Option<Ramp> = None;

    for (index, placed) in events.iter().enumerate() {
        if let Some(dynamic) = event_dynamic(placed.event) {
            current = dynamic_to_velocity(Some(dynamic)) as i32;
            ramp = None;
        } else if let Some(r) = &ramp {
            let span = (r.to.0 - r.from.0).max(1) as i64;
            let elapsed = (placed.tick - r.from.0) as i64;
            current = r.from.1 + ((r.to.1 - r.from.1) as i64 * elapsed / span) as i32;
        }

        if let Some(direction @ (Wedge::CrescendoStart | Wedge::DiminuendoStart)) =
            event_wedge(placed.event)
        {
            let stop = events[index + 1..]
                .iter()
                .position(|p| event_wedge(p.event) == Some(Wedge::Stop))
                .map(|offset| index + 1 + offset);
            ramp = stop.map(|stop| {
                let target = event_dynamic(events[stop].event)
                    .or_else(|| events.get(stop + 1).and_then(|p| event_dynamic(p.event)))
                    .map(|d| dynamic_to_velocity(Some(d)) as i32)
                    .unwrap_or(if direction == Wedge::CrescendoStart {
                        current + WEDGE_STEP
                    } else {
                        current - WEDGE_STEP
                    });
                Ramp {
                    from: (placed.tick, current),
                    to: (events[stop].tick, target.clamp(1, 127)),
                    stop,
                }
            });
        }

        velocities.push(current.clamp(1, 127) as u8);
        if ramp.as_ref().is_some_and(|r| r.stop == index) {
            ramp = None;
        }
    }
    velocities
}

fn part_track<'a>(part: &'a Part, timeline: &'a Timeline<'a>, channel_number: u8) -> Track<'a> {
    let channel = u4::new(channel_number);
    let mut raw: Vec<(u32, TrackEventKind<'a>)> = Vec::new();

    if let Some(name) = &part.name {
        raw.push((
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
        ));
    }

    let events = &timeline.events;
    let velocities = base_velocities(events);
    let mut open_note: Option<u8> = None;
    let mut pending_graces: Vec<usize> = Vec::new();
    let mut end = events.last().map_or(0, |p| p.tick + p.ticks);

    for (index, placed) in events.iter().enumerate() {
        if let Some(text) = &placed.chord_text {
            raw.push((
                placed.tick,
                TrackEventKind::Meta(MetaMessage::Text(text.as_bytes())),
            ));
        }
        if has_fermata(placed.event) {
            raw.push((
                placed.tick,
                TrackEventKind::Meta(MetaMessage::Marker(FERMATA_MARKER.as_bytes())),
            ));
        }
        if is_grace(placed.event) {
            pending_graces.push(index);
            continue;
        }

        let is_struck_note = match placed.event {
            Event::Note(note) => {
                !(matches!(note.tie, Tie::Stop | Tie::StartStop) && open_note.is_some())
            }
            Event::Chord(_) => true,
            Event::Rest(_) => false,
        };
        let steal = play_graces(
            &mut raw,
            channel,
            events,
            &velocities,
            &std::mem::take(&mut pending_graces),
            placed.tick,
            Some((placed.ticks, is_struck_note)),
        );
        let steal = if is_struck_note { steal } else { 0 };
        let onset = placed.tick + steal;
        let articulations = event_articulations(placed.event);
        let velocity = accented_velocity(velocities[index], &articulations);

        match placed.event {
            Event::Rest(_) => {}
            Event::Note(note) => {
                let key = midi_key(note);
                if is_struck_note && let Some(stale) = open_note.take() {
                    raw.push((placed.tick, note_off(channel, stale)));
                }
                let chain_ends_here = matches!(note.tie, Tie::None | Tie::Stop);
                if is_struck_note && chain_ends_here {
                    // A self-contained note: the only case where
                    // articulations shorten it and ornaments are realized.
                    let sounding = sounding_ticks(placed.ticks - steal, &articulations);
                    let segments = realize_ornament(
                        note.pitch,
                        sounding,
                        &note.ornaments,
                        placed.key_signature,
                    )
                    .unwrap_or_else(|| vec![(0, sounding, key)]);
                    for (offset, length, segment_key) in segments {
                        raw.push((onset + offset, note_on(channel, segment_key, velocity)));
                        raw.push((onset + offset + length, note_off(channel, segment_key)));
                    }
                } else {
                    if is_struck_note {
                        raw.push((onset, note_on(channel, key, velocity)));
                        open_note = Some(key);
                    }
                    if chain_ends_here {
                        let held = open_note.take().unwrap_or(key);
                        raw.push((placed.tick + placed.ticks, note_off(channel, held)));
                    }
                }
            }
            Event::Chord(notes) => {
                if let Some(stale) = open_note.take() {
                    raw.push((placed.tick, note_off(channel, stale)));
                }
                let sounding = sounding_ticks(placed.ticks - steal, &articulations);
                for note in notes {
                    let key = midi_key(note);
                    raw.push((onset, note_on(channel, key, velocity)));
                    raw.push((onset + sounding, note_off(channel, key)));
                }
            }
        }
    }

    if !pending_graces.is_empty() {
        let steal = play_graces(
            &mut raw,
            channel,
            events,
            &velocities,
            &pending_graces,
            end,
            None,
        );
        end += steal;
    }
    if let Some(key) = open_note.take() {
        raw.push((end, note_off(channel, key)));
    }

    raw.sort_by_key(|(tick, _)| *tick);
    to_track(raw, end)
}

/// Plays the grace events `graces` as a run starting at `tick`, before an
/// event `following` of `(notated ticks, is a struck note)` (see
/// [`grace_length`]), returning the total ticks the run takes.
fn play_graces<'a>(
    raw: &mut Vec<(u32, TrackEventKind<'a>)>,
    channel: u4,
    events: &[Placed<'_>],
    velocities: &[u8],
    graces: &[usize],
    tick: u32,
    following: Option<(u32, bool)>,
) -> u32 {
    if graces.is_empty() {
        return 0;
    }
    let length = grace_length(graces.len() as u32, following);
    for (position, &index) in graces.iter().enumerate() {
        let start = tick + position as u32 * length;
        let event = events[index].event;
        let velocity = accented_velocity(velocities[index], &event_articulations(event));
        for note in event_notes(event) {
            let key = midi_key(note);
            raw.push((start, note_on(channel, key, velocity)));
            raw.push((start + length, note_off(channel, key)));
        }
    }
    length * graces.len() as u32
}

/// Converts absolute-tick events (already sorted) into a delta-timed track
/// ending at `end`.
fn to_track(events: Vec<(u32, TrackEventKind<'_>)>, end: u32) -> Track<'_> {
    let mut track = Vec::with_capacity(events.len() + 1);
    let mut previous = 0u32;
    for (tick, kind) in events {
        let delta = tick.saturating_sub(previous);
        previous = previous.max(tick);
        track.push(TrackEvent {
            delta: u28::new(delta),
            kind,
        });
    }
    track.push(TrackEvent {
        delta: u28::new(end.saturating_sub(previous)),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    track
}

fn note_on<'a>(channel: u4, key: u8, velocity: u8) -> TrackEventKind<'a> {
    TrackEventKind::Midi {
        channel,
        message: MidiMessage::NoteOn {
            key: u7::new(key),
            vel: u7::new(velocity),
        },
    }
}

fn note_off<'a>(channel: u4, key: u8) -> TrackEventKind<'a> {
    TrackEventKind::Midi {
        channel,
        message: MidiMessage::NoteOff {
            key: u7::new(key),
            vel: u7::new(0),
        },
    }
}

fn midi_key(note: &Note) -> u8 {
    clamp_key(note.pitch.to_midi_number())
}

fn duration_ticks(duration: &Duration) -> u32 {
    (duration.quarter_length() * PPQ as f64).round().max(0.0) as u32
}

/// A grace note borrows no notated time (see [`crate::performance::grace_length`]
/// for how it's actually played).
fn note_ticks(note: &Note) -> u32 {
    if note.grace {
        0
    } else {
        duration_ticks(&note.duration)
    }
}
