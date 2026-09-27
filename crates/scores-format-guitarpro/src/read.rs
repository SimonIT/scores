use crate::convert::{
    DEFAULT_VELOCITY, chord_to_symbol, direction_to_navigation, ending_numbers_from_bitmask,
    gp_duration_to_model, grace_duration_to_model, note_midi_value, quantize_velocity,
    tremolo_marks_from_speed, velocity_to_dynamic,
};
use guitarpro::{BeatStatus, NoteType, SlideType, Song as GpSong};
use scores_core::{
    Articulation, Barline, BarlineType, Duration, Ending, Event, KeySignature, Measure,
    MeasureAttributes, Mode, Note, NoteValue, Ornament, Part, Pitch, Rest, Result, Score,
    ScoresError, Tie, TimeSignature, Voice,
};
use std::collections::HashMap;

pub fn read(data: &[u8]) -> Result<Score> {
    let mut song = GpSong::default();
    let read_result = match detect_version(data) {
        Some(3) => song.read_gp3(data),
        Some(4) => song.read_gp4(data),
        Some(5) => song.read_gp5(data),
        _ => {
            return Err(ScoresError::Unsupported(
                "only classic Guitar Pro 3/4/5 binary files are supported (not GP6 .gpx or GP7+ .gp)".into(),
            ));
        }
    };
    read_result.map_err(|e| ScoresError::Parse(format!("invalid Guitar Pro file: {e}")))?;

    let title = trimmed(&song.name);
    let composer = trimmed(&song.author);

    let parts = song
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| read_track(track, index, &song.measure_headers))
        .collect();

    Ok(Score {
        title,
        composer,
        parts,
    })
}

fn trimmed(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Classic GP3/4/5 files start with a length-prefixed version tag (e.g.
/// `"FICHIER GUITAR PRO v5.10"`); GP6/7 files are ZIP archives and don't
/// look like this at all.
fn detect_version(data: &[u8]) -> Option<u8> {
    let len = *data.first()? as usize;
    let tag = std::str::from_utf8(data.get(1..1 + len)?).ok()?;
    if tag.contains("v3") {
        Some(3)
    } else if tag.contains("v4") {
        Some(4)
    } else if tag.contains("v5") {
        Some(5)
    } else {
        None
    }
}

fn read_track(
    track: &guitarpro::Track,
    index: usize,
    headers: &[guitarpro::MeasureHeader],
) -> Part {
    let strings = &track.strings;

    // Each voice slot gets its own flat event stream (so a tie's "previous
    // note" reference stays valid across a measure boundary) plus a record
    // of how many events belonged to each measure, sliced out afterward.
    let voice_slots = track
        .measures
        .iter()
        .map(|m| m.voices.len())
        .max()
        .unwrap_or(0);
    let mut flat_events: Vec<Vec<Event>> = vec![Vec::new(); voice_slots];
    let mut boundaries: Vec<Vec<usize>> = vec![Vec::new(); voice_slots];
    let mut open_ties: Vec<HashMap<i8, (usize, Option<usize>)>> = vec![HashMap::new(); voice_slots];
    // The velocity each voice is currently playing at, so a dynamic is
    // only emitted where it changes (starting from Guitar Pro's default f,
    // which therefore never shows up as an explicit marking).
    let mut velocities: Vec<i16> = vec![DEFAULT_VELOCITY; voice_slots];

    for measure in &track.measures {
        for slot in 0..voice_slots {
            if let Some(voice) = measure.voices.get(slot) {
                for beat in &voice.beats {
                    let mut state = VoiceState {
                        events: &mut flat_events[slot],
                        open_ties: &mut open_ties[slot],
                        velocity: &mut velocities[slot],
                    };
                    read_beat(beat, strings, &mut state);
                }
            }
            boundaries[slot].push(flat_events[slot].len());
        }
    }

    // GP5 always stores two voices per measure; a trailing voice that never
    // holds anything in this track isn't a real voice.
    let used_slots = (0..voice_slots)
        .rev()
        .find(|&slot| !flat_events[slot].is_empty())
        .map_or(voice_slots.min(1), |slot| slot + 1);

    let mut current_time: Option<TimeSignature> = None;
    let mut current_key: Option<KeySignature> = None;
    let mut measures = Vec::with_capacity(track.measures.len());
    let endings = endings_from_headers(headers);

    for (i, measure) in track.measures.iter().enumerate() {
        let time_signature = TimeSignature {
            beats: measure.time_signature.numerator.max(1) as u8,
            beat_type: measure.time_signature.denominator.value.max(1) as u8,
        };
        let key_signature = KeySignature {
            fifths: measure.key_signature.key,
            mode: if measure.key_signature.is_minor {
                Mode::Minor
            } else {
                Mode::Major
            },
        };

        let voices: Vec<Voice> = (0..used_slots)
            .map(|slot| {
                let start = if i == 0 { 0 } else { boundaries[slot][i - 1] };
                let end = boundaries[slot][i];
                Voice {
                    number: (slot + 1) as u8,
                    events: flat_events[slot][start..end].to_vec(),
                }
            })
            .collect();
        let has_any_events = voices.iter().any(|v| !v.events.is_empty());
        let voices = if has_any_events {
            voices
        } else {
            vec![Voice {
                number: 1,
                events: vec![Event::Rest(Rest::new(Duration::new(NoteValue::Whole)))],
            }]
        };

        let changed = current_time != Some(time_signature) || current_key != Some(key_signature);
        let attributes = if changed {
            current_time = Some(time_signature);
            current_key = Some(key_signature);
            MeasureAttributes {
                time_signature: Some(time_signature),
                key_signature: Some(key_signature),
                clef: None,
                tempo: None,
            }
        } else {
            MeasureAttributes::default()
        };

        let header = headers.get(i);
        let mut barline = header.map(barline_from_header).unwrap_or_default();
        barline.ending = endings.get(i).cloned().flatten();

        measures.push(Measure {
            number: (i + 1) as u32,
            attributes,
            voices,
            barline,
            navigation: header
                .and_then(|h| h.direction.as_ref())
                .map(direction_to_navigation)
                .into_iter()
                .collect(),
        });
    }

    Part {
        id: format!("P{}", index + 1),
        name: trimmed(&track.name),
        measures,
    }
}

/// `repeat_close >= 0` marks a repeat-end; the crate stores the number of
/// *repeats* there (the GP5 reader already subtracts one from the file's
/// play count, and the crate's own converters add it back), so the play
/// count is one more - floored at 2, since "repeat" implies at least a
/// second playthrough. Guitar Pro's only barline style beyond the implicit
/// ones is `double_bar`, a light-light double barline.
fn barline_from_header(header: &guitarpro::MeasureHeader) -> Barline {
    Barline {
        kind: if header.double_bar {
            BarlineType::LightLight
        } else {
            BarlineType::Regular
        },
        repeat_start: header.repeat_open,
        repeat_end: (header.repeat_close >= 0).then(|| (header.repeat_close as u32 + 1).max(2)),
        ending: None,
    }
}

/// Guitar Pro marks *every* measure under an ending bracket with the
/// bracket's `repeat_alternative` bitmask (bit N = ending N+1). Each run of
/// consecutive measures sharing a mask becomes one bracket on its first
/// measure, spanning the run. Guitar Pro draws no hook distinction, so the
/// bracket counts as closed when the run ends with a repeat-end (it leads
/// back) and open otherwise.
fn endings_from_headers(headers: &[guitarpro::MeasureHeader]) -> Vec<Option<Ending>> {
    let mut endings = vec![None; headers.len()];
    let mut i = 0;
    while i < headers.len() {
        let mask = headers[i].repeat_alternative;
        let mut end = i + 1;
        while end < headers.len() && headers[end].repeat_alternative == mask {
            end += 1;
        }
        if mask != 0 {
            endings[i] = Some(Ending {
                numbers: ending_numbers_from_bitmask(mask),
                measures: (end - i) as u32,
                closed: headers[end - 1].repeat_close >= 0,
            });
        }
        i = end;
    }
    endings
}

/// Per-voice-slot reading state carried from beat to beat.
struct VoiceState<'a> {
    events: &'a mut Vec<Event>,
    open_ties: &'a mut HashMap<i8, (usize, Option<usize>)>,
    velocity: &'a mut i16,
}

fn read_beat(beat: &guitarpro::Beat, strings: &[(i8, i8)], state: &mut VoiceState) {
    let duration = gp_duration_to_model(&beat.duration);
    let chord_symbol = beat.effect.chord.as_ref().and_then(chord_to_symbol);
    let events = &mut *state.events;

    if beat.status == BeatStatus::Rest || beat.notes.is_empty() {
        let mut rest = Rest::new(duration);
        rest.chord_symbol = chord_symbol;
        events.push(Event::Rest(rest));
        return;
    }

    let built: Vec<(i8, &guitarpro::Note, Note)> = beat
        .notes
        .iter()
        .filter_map(|gp_note| {
            let midi = note_midi_value(gp_note, strings)?;
            let mut note = Note::new(Pitch::from_midi_number(midi, false), duration, Tie::None);
            apply_note_effect(&mut note, gp_note, beat);
            Some((gp_note.string, gp_note, note))
        })
        .collect();

    if built.is_empty() {
        let mut rest = Rest::new(duration);
        rest.chord_symbol = chord_symbol;
        events.push(Event::Rest(rest));
        return;
    }

    // A Guitar Pro grace note is an effect on the note it leads into (same
    // string, its own fret); ours is a separate, timeless event before it.
    let grace_notes: Vec<Note> = beat
        .notes
        .iter()
        .filter_map(|gp_note| {
            let grace = gp_note.effect.grace.as_ref()?;
            let tuning = strings
                .get(usize::try_from(gp_note.string).ok()?.checked_sub(1)?)?
                .1;
            let midi = i32::from(grace.fret.max(0)) + i32::from(tuning);
            let mut note = Note::new(
                Pitch::from_midi_number(midi, false),
                grace_duration_to_model(grace.duration),
                Tie::None,
            );
            note.grace = true;
            Some(note)
        })
        .collect();
    match grace_notes.len() {
        0 => {}
        1 => events.push(Event::Note(grace_notes.into_iter().next().unwrap())),
        _ => events.push(Event::Chord(grace_notes)),
    }

    let event_index = events.len();
    let is_chord = built.len() > 1;
    let mut notes_only: Vec<Note> = built.iter().map(|(_, _, n)| n.clone()).collect();

    // Tied continuations just sustain the previous note, so they don't
    // establish a new dynamic level.
    let velocity = built
        .iter()
        .filter(|(_, gp_note, _)| !matches!(gp_note.kind, NoteType::Tie))
        .map(|(_, gp_note, _)| quantize_velocity(gp_note.velocity))
        .max();
    if let Some(velocity) = velocity
        && velocity != *state.velocity
    {
        *state.velocity = velocity;
        notes_only[0].dynamic = Some(velocity_to_dynamic(velocity));
    }

    let mut event = if is_chord {
        Event::Chord(notes_only)
    } else {
        Event::Note(notes_only.into_iter().next().unwrap())
    };
    event.set_chord_symbol(chord_symbol);
    events.push(event);

    for (slot, (string, gp_note, _)) in built.iter().enumerate() {
        let chord_slot = is_chord.then_some(slot);
        if matches!(gp_note.kind, NoteType::Tie) {
            if let Some(previous) = state.open_ties.get(string).copied() {
                let previous_tie = tie_mut(events, previous);
                *previous_tie = if *previous_tie == Tie::Stop {
                    Tie::StartStop
                } else {
                    Tie::Start
                };
            }
            *tie_mut(events, (event_index, chord_slot)) = Tie::Stop;
        }
        state.open_ties.insert(*string, (event_index, chord_slot));
    }
}

/// Maps the note-effect flags this crate models onto our vocabulary:
/// accents/staccato directly, the four "slide in/out" flavours as the jazz
/// articulations they're notated as (scoop/plop/doit/falloff), trill,
/// tremolo picking (as tremolo slashes) and vibrato (as a wavy line).
/// `bend`/`harmonic`/shift and legato slides/`hammer`/`palm_mute`/
/// `let_ring`/`ghost_note`/fingerings are guitar-specific techniques with
/// no equivalent here and are left unmapped. Grace notes are handled by
/// the caller, since they become events of their own.
fn apply_note_effect(note: &mut Note, gp_note: &guitarpro::Note, beat: &guitarpro::Beat) {
    let effect = &gp_note.effect;
    if effect.accentuated_note {
        note.articulations.push(Articulation::Accent);
    }
    if effect.heavy_accentuated_note {
        note.articulations.push(Articulation::StrongAccent);
    }
    if effect.staccato {
        note.articulations.push(Articulation::Staccato);
    }
    for slide in &effect.slides {
        let articulation = match slide {
            SlideType::IntoFromBelow => Articulation::Scoop,
            SlideType::IntoFromAbove => Articulation::Plop,
            SlideType::OutUpWards => Articulation::Doit,
            SlideType::OutDownwards => Articulation::Falloff,
            _ => continue,
        };
        note.articulations.push(articulation);
    }
    if effect.trill.is_some() {
        note.ornaments.push(Ornament::TrillMark);
    }
    if let Some(tremolo) = &effect.tremolo_picking {
        note.ornaments
            .push(Ornament::Tremolo(tremolo_marks_from_speed(
                tremolo.duration.value,
            )));
    }
    if effect.vibrato || beat.effect.vibrato {
        note.ornaments.push(Ornament::WavyLine);
    }
}

fn tie_mut(events: &mut [Event], (event_index, chord_slot): (usize, Option<usize>)) -> &mut Tie {
    match &mut events[event_index] {
        Event::Note(n) => &mut n.tie,
        Event::Chord(notes) => {
            &mut notes[chord_slot.expect("chord slot required for Event::Chord")].tie
        }
        Event::Rest(_) => unreachable!("ties are only ever recorded for Note/Chord events"),
    }
}
