use crate::convert::{
    DEFAULT_VELOCITY, dynamic_velocities, ending_bitmask_from_numbers, model_duration_to_gp,
    model_duration_to_grace, navigation_to_direction, symbol_to_chord, tremolo_speed_from_marks,
};
use guitarpro::model::legacy::beat::BeatEffects;
use guitarpro::model::legacy::effects::{GraceEffect, TremoloPickingEffect, TrillEffect};
use guitarpro::model::legacy::key_signature::Duration as GpDuration;
use guitarpro::model::legacy::note::NoteEffect;
use guitarpro::{
    BeatStatus, KeySignature as GpKeySignature, MeasureHeader, NoteType, SlideType, Song as GpSong,
    TimeSignature as GpTimeSignature, Track, Voice, VoiceDirection,
};
use scores_core::{
    Articulation, Barline, BarlineType, Event, KeySignature, Measure, Mode, Note, Ornament, Part,
    Result, Score, ScoresError, Tie, TimeSignature,
};
use std::collections::HashSet;

/// Frets beyond the neck are not modeled, so pitches out of a string's
/// 0-24 range are clamped to the nearest playable fret on it.
const MAX_FRET: i32 = 24;

#[allow(clippy::field_reassign_with_default)]
pub fn write(score: &Score) -> Result<Vec<u8>> {
    if score.parts.is_empty() {
        return Err(ScoresError::Unsupported(
            "cannot write an empty score to Guitar Pro format".into(),
        ));
    }

    let mut song = GpSong::default();
    song.name = score.title.clone().unwrap_or_default();
    song.author = score.composer.clone().unwrap_or_default();
    // `Song::default()` leaves `lyrics.lines` and `channels` empty, but the
    // writer unconditionally indexes 5 lyric lines and each track's
    // `channel_index` into `channels` - fill both in so it doesn't panic.
    song.lyrics.lines = (0..5u8).map(|i| (i, 1u16, String::new())).collect();
    song.channels = (0..64u8)
        .map(|i| guitarpro::audio::midi::MidiChannel {
            channel: i,
            effect_channel: i,
            ..guitarpro::audio::midi::MidiChannel::default()
        })
        .collect();
    song.tempo = score
        .parts
        .first()
        .and_then(|p| p.measures.first())
        .and_then(|m| m.attributes.tempo)
        .map(|t| t.bpm.round() as i16)
        .unwrap_or(120);

    let measure_count = score
        .parts
        .iter()
        .map(|p| p.measures.len())
        .max()
        .unwrap_or(0);
    let ending_masks = ending_masks(score, measure_count);
    song.measure_headers = (0..measure_count)
        .map(|index| build_measure_header(score, index, ending_masks[index]))
        .collect();
    song.tracks = score
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| build_track(part, measure_count, index))
        .collect();

    // NOTE: (5, 1, 0) does not round-trip in `guitarpro` 0.4.3 (its
    // version-gated ">5.0.0" fields - RSE master effect / hide_tempo /
    // directions - appear to have a write/read asymmetry that corrupts
    // page_setup on read). (5, 0, 0) does not hit that path.
    song.write((5, 0, 0), None)
        .map_err(|e| ScoresError::Write(format!("failed to serialize Guitar Pro file: {e}")))
}

fn measure_attributes_at(score: &Score, index: usize) -> (TimeSignature, KeySignature) {
    // Every part carries its own (sparse) attributes; take the first part
    // that actually has a measure here, and the closest-preceding attribute
    // change on it, defaulting to 4/4 C major if nothing was ever set.
    for part in &score.parts {
        let mut time_signature = None;
        let mut key_signature = None;
        for measure in part.measures.iter().take(index + 1) {
            time_signature = measure.attributes.time_signature.or(time_signature);
            key_signature = measure.attributes.key_signature.or(key_signature);
        }
        if part.measures.len() > index {
            return (
                time_signature.unwrap_or_default(),
                key_signature.unwrap_or_default(),
            );
        }
    }
    (TimeSignature::default(), KeySignature::default())
}

/// Repeats/endings are measure-level in our model but track-shared
/// (`MeasureHeader`) in Guitar Pro's, so - like time/key signature above -
/// the first part with a measure at this index wins.
fn barline_at(score: &Score, index: usize) -> Barline {
    score
        .parts
        .iter()
        .find_map(|part| part.measures.get(index))
        .map(|m| m.barline.clone())
        .unwrap_or_default()
}

/// Guitar Pro marks every measure under an ending bracket with its
/// bitmask, so each bracket's mask is filled across the measures it spans
/// (clamped to the score). Whether it's closed isn't stored: the reader
/// infers that from a repeat-end on its last measure.
fn ending_masks(score: &Score, measure_count: usize) -> Vec<u8> {
    let mut masks = vec![0; measure_count];
    for start in 0..measure_count {
        if let Some(ending) = barline_at(score, start).ending {
            let end = (start + ending.measures.max(1) as usize).min(measure_count);
            masks[start..end].fill(ending_bitmask_from_numbers(&ending.numbers));
        }
    }
    masks
}

/// Navigation marks are track-shared (`MeasureHeader::direction`) in Guitar
/// Pro, so the first part carrying one at this measure wins. A header holds
/// a single direction, so only a measure's first mark is kept.
fn navigation_at(score: &Score, index: usize) -> Option<scores_core::Navigation> {
    score
        .parts
        .iter()
        .find_map(|part| part.measures.get(index)?.navigation.first().copied())
}

fn build_measure_header(score: &Score, index: usize, ending_mask: u8) -> MeasureHeader {
    let (time_signature, key_signature) = measure_attributes_at(score, index);
    let barline = barline_at(score, index);
    MeasureHeader {
        number: (index + 1) as u16,
        time_signature: to_gp_time_signature(time_signature),
        key_signature: to_gp_key_signature(key_signature),
        repeat_open: barline.repeat_start,
        // The crate stores the number of *repeats* (play count - 1).
        repeat_close: barline
            .repeat_end
            .map(|times| times.saturating_sub(1).clamp(1, 126) as i8)
            .unwrap_or(-1),
        repeat_alternative: ending_mask,
        // Guitar Pro's only explicit barline style: approximate the
        // double-line styles with it and leave the rest regular (repeat
        // and final barlines are drawn implicitly).
        double_bar: matches!(
            barline.kind,
            BarlineType::LightLight | BarlineType::HeavyHeavy
        ),
        // GP5 allows each sign once per song: if several measures carry
        // the same one, the crate's writer keeps the last.
        direction: navigation_at(score, index).map(navigation_to_direction),
        ..MeasureHeader::default()
    }
}

fn to_gp_time_signature(ts: TimeSignature) -> GpTimeSignature {
    GpTimeSignature {
        numerator: ts.beats as i8,
        denominator: GpDuration {
            value: ts.beat_type as u16,
            ..GpDuration::default()
        },
        beams: vec![2, 2, 2, 2],
    }
}

fn to_gp_key_signature(key: KeySignature) -> GpKeySignature {
    GpKeySignature {
        key: key.fifths,
        is_minor: key.mode == Mode::Minor,
    }
}

#[allow(clippy::field_reassign_with_default)]
fn build_track(part: &Part, measure_count: usize, channel_index: usize) -> Track {
    let mut track = Track::default();
    track.name = part.name.clone().unwrap_or_else(|| part.id.clone());
    track.channel_index = channel_index % 64;
    let strings = track.strings.clone();
    // Each voice's current velocity carries across measures, so a dynamic
    // keeps applying until the next one.
    let mut velocities = [DEFAULT_VELOCITY; 2];

    track.measures = (0..measure_count)
        .map(|index| build_measure(part.measures.get(index), index, &strings, &mut velocities))
        .collect();
    track
}

/// Guitar Pro always stores exactly 2 voices per measure (`MAX_VOICES`);
/// omitting the second one desyncs the reader for every measure after it.
/// Our model's voices beyond the first 2 (rare) are dropped.
fn build_measure(
    measure: Option<&Measure>,
    index: usize,
    strings: &[(i8, i8)],
    velocities: &mut [i16; 2],
) -> guitarpro::Measure {
    let attrs = measure.map(|m| &m.attributes);
    let time_signature = attrs.and_then(|a| a.time_signature).unwrap_or_default();
    let key_signature = attrs.and_then(|a| a.key_signature).unwrap_or_default();

    let mut voices: Vec<Voice> = match measure {
        Some(measure) => measure
            .voices
            .iter()
            .take(2)
            .zip(velocities.iter_mut())
            .map(|(voice, velocity)| Voice {
                measure_index: index as i16,
                beats: build_beats(&voice.events, strings, velocity),
                directions: VoiceDirection::None,
            })
            .collect(),
        None => vec![Voice {
            measure_index: index as i16,
            beats: vec![whole_rest_beat()],
            directions: VoiceDirection::None,
        }],
    };
    while voices.len() < 2 {
        voices.push(Voice {
            measure_index: index as i16,
            beats: Vec::new(),
            directions: VoiceDirection::None,
        });
    }

    guitarpro::Measure {
        number: index + 1,
        time_signature: to_gp_time_signature(time_signature),
        key_signature: to_gp_key_signature(key_signature),
        voices,
        ..guitarpro::Measure::default()
    }
}

fn whole_rest_beat() -> guitarpro::Beat {
    guitarpro::Beat {
        duration: GpDuration::default(),
        status: BeatStatus::Rest,
        ..guitarpro::Beat::default()
    }
}

/// Converts one voice's events to beats. Grace notes aren't beats of their
/// own in Guitar Pro but an effect on the note they lead into, so a grace
/// event is held back and attached to the next sounding event (Guitar Pro
/// allows one grace per note: of several consecutive grace events only the
/// last is kept, and graces before a rest or the end of the measure are
/// dropped).
fn build_beats(events: &[Event], strings: &[(i8, i8)], velocity: &mut i16) -> Vec<guitarpro::Beat> {
    let mut beats = Vec::with_capacity(events.len());
    let mut pending_grace: Vec<&Note> = Vec::new();
    for event in events {
        let notes: Vec<&Note> = match event {
            Event::Note(note) => vec![note],
            Event::Chord(notes) => notes.iter().collect(),
            Event::Rest(_) => Vec::new(),
        };
        // A dynamic on a grace note still sets the level for what follows.
        let dynamic = notes.iter().find_map(|n| n.dynamic.as_ref());
        let (note_velocity, next_velocity) = dynamic
            .map(|d| dynamic_velocities(d, *velocity))
            .unwrap_or((*velocity, *velocity));
        *velocity = next_velocity;

        if notes.first().is_some_and(|n| n.grace) {
            pending_grace = notes;
            continue;
        }
        let mut beat = build_beat(event, strings, note_velocity);
        attach_grace_notes(
            &mut beat,
            &std::mem::take(&mut pending_grace),
            strings,
            note_velocity,
        );
        beats.push(beat);
    }
    beats
}

fn build_beat(event: &Event, strings: &[(i8, i8)], velocity: i16) -> guitarpro::Beat {
    let duration = model_duration_to_gp(&event.duration());
    let mut used = HashSet::new();

    let notes: Vec<guitarpro::Note> = match event {
        Event::Rest(_) => Vec::new(),
        Event::Note(note) => assign_string(note, strings, &mut used, velocity)
            .into_iter()
            .collect(),
        Event::Chord(notes) => notes
            .iter()
            .filter_map(|note| assign_string(note, strings, &mut used, velocity))
            .collect(),
    };

    let status = if notes.is_empty() {
        BeatStatus::Rest
    } else {
        BeatStatus::Normal
    };
    guitarpro::Beat {
        duration,
        notes,
        status,
        effect: BeatEffects {
            chord: event.chord_symbol().map(symbol_to_chord),
            ..BeatEffects::default()
        },
        ..guitarpro::Beat::default()
    }
}

/// Gives each grace note to the first not-yet-graced note of `beat` whose
/// string can play it (a Guitar Pro grace shares its main note's string);
/// grace notes no string can reach are dropped.
fn attach_grace_notes(
    beat: &mut guitarpro::Beat,
    grace_notes: &[&Note],
    strings: &[(i8, i8)],
    velocity: i16,
) {
    for grace in grace_notes {
        let midi = grace.pitch.to_midi_number();
        let target = beat.notes.iter_mut().find_map(|note| {
            if note.effect.grace.is_some() {
                return None;
            }
            let index = usize::try_from(note.string).ok()?.checked_sub(1)?;
            let fret = midi - i32::from(strings.get(index)?.1);
            (0..=MAX_FRET).contains(&fret).then_some((note, fret))
        });
        if let Some((note, fret)) = target {
            note.effect.grace = Some(GraceEffect {
                fret: fret as i8,
                duration: model_duration_to_grace(&grace.duration),
                velocity,
                ..GraceEffect::default()
            });
        }
    }
}

/// Picks the highest string (thinnest/highest-pitched first, matching the
/// default tuning's declaration order) that can play `note` at a fret in
/// `0..=24`, falling back to the closest clamped fret on whichever
/// available string comes nearest.
fn assign_string(
    note: &Note,
    strings: &[(i8, i8)],
    used: &mut HashSet<i8>,
    velocity: i16,
) -> Option<guitarpro::Note> {
    let midi = note.pitch.to_midi_number();
    let mut fallback: Option<(i8, i32, i32)> = None;

    for &(string_number, tuning) in strings {
        if used.contains(&string_number) {
            continue;
        }
        let fret = midi - i32::from(tuning);
        if (0..=MAX_FRET).contains(&fret) {
            used.insert(string_number);
            return Some(make_note(string_number, fret, note, velocity));
        }
        let clamped = fret.clamp(0, MAX_FRET);
        let distance = (fret - clamped).abs();
        if fallback.is_none_or(|(_, _, best)| distance < best) {
            fallback = Some((string_number, clamped, distance));
        }
    }

    fallback.map(|(string_number, fret, _)| {
        used.insert(string_number);
        make_note(string_number, fret, note, velocity)
    })
}

/// Maps articulations/ornaments onto the note-effect flags this crate
/// models (the inverse of the reader's `apply_note_effect`); everything
/// else in our vocabulary has no guitar-tab equivalent and is dropped.
fn make_note(string: i8, fret: i32, note: &Note, velocity: i16) -> guitarpro::Note {
    let slides = note
        .articulations
        .iter()
        .filter_map(|articulation| match articulation {
            Articulation::Scoop => Some(SlideType::IntoFromBelow),
            Articulation::Plop => Some(SlideType::IntoFromAbove),
            Articulation::Doit => Some(SlideType::OutUpWards),
            Articulation::Falloff => Some(SlideType::OutDownwards),
            _ => None,
        })
        .collect();
    let has_trill = note.ornaments.contains(&Ornament::TrillMark);
    let tremolo = note.ornaments.iter().find_map(|o| match o {
        Ornament::Tremolo(marks) => Some(*marks),
        _ => None,
    });
    let effect = NoteEffect {
        accentuated_note: note.articulations.contains(&Articulation::Accent),
        heavy_accentuated_note: note.articulations.contains(&Articulation::StrongAccent),
        staccato: note.articulations.contains(&Articulation::Staccato),
        slides,
        // A trill alternates with a higher fret on the same string; our
        // trill mark doesn't say by how much, so assume a whole step. Its
        // `duration` must be one of the crate's recognized trill speeds
        // (16th/32nd/64th); 16th is the common default.
        trill: has_trill.then(|| TrillEffect {
            fret: (fret + 2).min(MAX_FRET) as i8,
            duration: GpDuration {
                value: 16,
                ..GpDuration::default()
            },
        }),
        tremolo_picking: tremolo.map(|marks| TremoloPickingEffect {
            duration: GpDuration {
                value: tremolo_speed_from_marks(marks),
                ..GpDuration::default()
            },
        }),
        // A wavy line alongside a trill mark is the trill's own extension
        // line, not vibrato.
        vibrato: !has_trill && note.ornaments.contains(&Ornament::WavyLine),
        ..NoteEffect::default()
    };
    guitarpro::Note {
        value: fret as i16,
        velocity,
        string,
        effect,
        kind: if matches!(note.tie, Tie::Stop | Tie::StartStop) {
            NoteType::Tie
        } else {
            NoteType::Normal
        },
        ..guitarpro::Note::default()
    }
}
