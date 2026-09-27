//! Converts a `guitarpro::model::optimized::LoadedScore` (parsed from an
//! `.mscz` archive) into a [`Score`]. See the module docs in [`crate`] for
//! what the `guitarpro` crate's mscz converter actually round-trips today;
//! everything it drops is recovered by a second pass over the raw XML
//! ([`crate::mscx::scan`]) and overlaid here.

use crate::convert::{clef_to_model, duration_to_model, key_signature_to_model, pitch_to_model};
use crate::mscx::{self, BeatExtras, MeasureExtras};
use guitarpro::model::optimized::beat::Beat as OptBeat;
use guitarpro::model::optimized::timeline::JumpKind;
use scores_core::{
    Barline, Clef, Event, Measure, MeasureAttributes, Note, Part, Result, Score, ScoresError,
    Tempo, Tie, TimeSignature, Voice,
};

pub fn read(data: &[u8]) -> Result<Score> {
    let file = guitarpro::read_mscz_bytes(data)
        .map_err(|e| ScoresError::Parse(format!("invalid MuseScore file: {e}")))?;
    let outcome = guitarpro::convert::mscz::mscx_to_loaded_score(&file.mscx);
    let loaded = outcome.score.score;
    // Indexed like `loaded`: staff = track, then measure/voice/beat.
    let extras = mscx::scan(&file.mscx.raw_xml)?;

    let title = (!loaded.metadata.title.is_empty()).then(|| loaded.metadata.title.clone());
    let composer = loaded.metadata.composer.clone();

    let timeline_attrs: Vec<MeasureAttributes> = loaded
        .timeline
        .iter()
        .map(|def| MeasureAttributes {
            time_signature: def.time_signature.map(|ts| TimeSignature {
                beats: ts.numerator,
                beat_type: ts.denominator,
            }),
            key_signature: def.key_signature.map(key_signature_to_model),
            clef: None,
            tempo: def.tempo.map(|bpm| Tempo { bpm }),
        })
        .collect();

    // Only plain repeat-open/repeat-close barlines are populated by the
    // converter (see module docs); endings, barline style and navigation
    // come from the raw-XML scan below.
    let repeat_starts: Vec<bool> = loaded
        .timeline
        .iter()
        .map(|def| {
            def.navigation
                .iter()
                .any(|ev| ev.kind == JumpKind::RepeatOpen)
        })
        .collect();
    let repeat_ends: Vec<Option<u32>> = loaded
        .timeline
        .iter()
        .map(|def| {
            def.navigation
                .iter()
                .find(|ev| ev.kind == JumpKind::RepeatClose)
                .map(|ev| ev.repeat_count.unwrap_or(2) as u32)
        })
        .collect();

    let mut parts = Vec::new();
    // Voltas and jumps/markers are system-wide, stored in the first staff.
    let system_measures: &[MeasureExtras] = extras
        .first()
        .map(|staff| staff.measures.as_slice())
        .unwrap_or_default();

    for (track_index, track) in loaded.tracks.iter().enumerate() {
        let staff_measures: &[MeasureExtras] = extras
            .get(track_index)
            .map(|staff| staff.measures.as_slice())
            .unwrap_or_default();
        let name = loaded
            .instruments
            .get(track.instrument.0 as usize)
            .map(|instrument| instrument.name.clone())
            .filter(|name| !name.is_empty());
        let clef = track
            .staves
            .first()
            .and_then(|staff_id| loaded.staves.get(staff_id.0 as usize))
            .map(|staff| clef_to_model(staff.clef));

        let mut measures = Vec::with_capacity(loaded.timeline.len());
        for (index, def) in loaded.timeline.iter().enumerate() {
            let mut attributes = timeline_attrs[index].clone();
            if index == 0 {
                attributes.clef = Some(clef.unwrap_or(Clef::Treble));
            }

            let measure_extras = staff_measures.get(index);
            let system_extras = system_measures.get(index);
            let barline = Barline {
                kind: measure_extras.map(|m| m.barline).unwrap_or_default(),
                repeat_start: repeat_starts[index],
                repeat_end: repeat_ends[index],
                ending: system_extras.and_then(|m| m.ending.clone()),
            };

            let mut voices: Vec<Voice> = Vec::new();
            if let Some(measure_data) = track.measures.get(&def.index) {
                let mut voice_ids: Vec<&u8> = measure_data.voices.keys().collect();
                voice_ids.sort();
                for (position, &voice_id) in voice_ids.into_iter().enumerate() {
                    let voice = &measure_data.voices[&voice_id];
                    let beat_extras = measure_extras.and_then(|m| m.voices.get(position));
                    let events = voice
                        .beats
                        .iter()
                        .enumerate()
                        .map(|(b, beat)| {
                            beat_to_event(beat, beat_extras.and_then(|beats| beats.get(b)))
                        })
                        .collect();
                    voices.push(Voice {
                        number: voice_id + 1,
                        events,
                    });
                }
            }
            if voices.is_empty() {
                voices.push(Voice {
                    number: 1,
                    events: Vec::new(),
                });
            }

            measures.push(Measure {
                number: (index + 1) as u32,
                attributes,
                voices,
                barline,
                navigation: system_extras
                    .map(|m| m.navigation.clone())
                    .unwrap_or_default(),
            });
        }

        parts.push(Part {
            id: format!("P{}", parts.len() + 1),
            name,
            measures,
        });
    }

    if parts.is_empty() {
        return Err(ScoresError::Parse(
            "MuseScore file contains no tracks".into(),
        ));
    }

    Ok(Score {
        title,
        composer,
        parts,
    })
}

/// Tuplets are never populated by the converter (see module docs), and a
/// "ghost" note (`pitch: None`) is dropped from whatever chord it was part
/// of. `extras` (from the raw-XML scan) supplies grace/fermata/marks/
/// dynamics/hairpins/chord symbols: per-chord marks go on the chord's first
/// note, the grace flag on every note.
fn beat_to_event(beat: &OptBeat, extras: Option<&BeatExtras>) -> Event {
    let duration = duration_to_model(&beat.duration);
    let notes: Vec<Note> = beat
        .notes
        .iter()
        .filter_map(|note| {
            let pitch = note.pitch?;
            let tie = match note.tie {
                Some(guitarpro::model::optimized::note::TieType::Start) => Tie::Start,
                Some(guitarpro::model::optimized::note::TieType::End) => Tie::Stop,
                None => Tie::None,
            };
            Some(Note::new(pitch_to_model(pitch), duration, tie))
        })
        .collect();

    let mut event = match notes.len() {
        0 => Event::Rest(scores_core::Rest::new(duration)),
        1 => Event::Note(notes.into_iter().next().expect("checked len == 1")),
        _ => Event::Chord(notes),
    };
    if let Some(extras) = extras {
        apply_extras(&mut event, extras);
    }
    event
}

fn apply_extras(event: &mut Event, extras: &BeatExtras) {
    event.set_chord_symbol(extras.chord_symbol.clone());
    let notes: &mut [Note] = match event {
        Event::Note(note) => std::slice::from_mut(note),
        Event::Chord(notes) => notes,
        Event::Rest(rest) => {
            rest.fermata = extras.fermata;
            return;
        }
    };
    for note in notes.iter_mut() {
        note.grace = extras.grace;
    }
    if let Some(first) = notes.first_mut() {
        first.fermata = extras.fermata;
        first.articulations = extras.articulations.clone();
        first.ornaments = extras.ornaments.clone();
        first.dynamic = extras.dynamic.clone();
        first.wedge = extras.wedge;
    }
}
