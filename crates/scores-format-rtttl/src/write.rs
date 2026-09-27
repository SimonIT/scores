use scores_core::{
    Duration, Event, Note, NoteValue, Pitch, Result, Score, Step, Tuplet, playback_order,
};

/// RTTTL is monophonic and has no bar lines or multi-part concept, so only
/// the first part's first voice is written, and only the first note of a
/// chord. Grace notes are dropped, since every RTTTL tone takes real time.
/// Repeats, endings and D.C./D.S./Coda/Fine jumps are unrolled into playback
/// order (see [`playback_order`]), since a ringtone is a single performance.
pub fn write(score: &Score) -> Result<Vec<u8>> {
    let events: Vec<&Event> = score
        .parts
        .first()
        .map(|part| {
            playback_order(&part.measures)
                .into_iter()
                .map(|index| &part.measures[index])
                .flat_map(|m| {
                    m.voices
                        .first()
                        .into_iter()
                        .flat_map(|voice| voice.events.iter())
                        .filter(|event| !is_grace(event))
                })
                .collect()
        })
        .unwrap_or_default();

    let name: String = score
        .title
        .as_deref()
        .unwrap_or("untitled")
        .chars()
        .take(10)
        .collect();
    let tones: Vec<String> = events.into_iter().map(tone_token).collect();

    Ok(format!("{name}:d=4,o=5,b=120:{}", tones.join(",")).into_bytes())
}

fn is_grace(event: &Event) -> bool {
    match event {
        Event::Note(note) => note.grace,
        Event::Chord(notes) => notes.first().is_some_and(|n| n.grace),
        Event::Rest(_) => false,
    }
}

fn tone_token(event: &Event) -> String {
    match event {
        Event::Note(note) => note_token(note),
        Event::Rest(rest) => format!(
            "{}p{}",
            duration_digits(rest.duration.base),
            modifier_suffix(&rest.duration)
        ),
        // RTTTL can't represent simultaneous notes - keep only the first (typically highest) voice.
        Event::Chord(notes) => notes.first().map(note_token).unwrap_or_default(),
    }
}

fn note_token(note: &Note) -> String {
    // Re-spell via MIDI number rather than trusting the original step/alter,
    // since RTTTL has no flat sign and no double sharps/flats.
    let respelled = Pitch::from_midi_number(note.pitch.to_midi_number(), false);
    format!(
        "{}{}{}{}{}",
        duration_digits(note.duration.base),
        step_letter(respelled.step),
        if respelled.alter > 0 { "#" } else { "" },
        (respelled.octave - 1).clamp(0, 8),
        modifier_suffix(&note.duration),
    )
}

/// RTTTL only allows a duration denominator of 1/2/4/8/16/32 - a 64th note
/// is rounded up to a 32nd.
fn duration_digits(value: NoteValue) -> u8 {
    match value {
        NoteValue::Whole => 1,
        NoteValue::Half => 2,
        NoteValue::Quarter => 4,
        NoteValue::Eighth => 8,
        NoteValue::Sixteenth => 16,
        NoteValue::ThirtySecond | NoteValue::SixtyFourth => 32,
    }
}

fn step_letter(step: Step) -> char {
    match step {
        Step::A => 'a',
        Step::B => 'b',
        Step::C => 'c',
        Step::D => 'd',
        Step::E => 'e',
        Step::F => 'f',
        Step::G => 'g',
    }
}

/// A trailing dot/double-dot, or (for the common 3:2 case only - this
/// crate's one tuplet extension) `&` for a triplet. Any other tuplet ratio
/// is dropped rather than approximated.
fn modifier_suffix(duration: &Duration) -> &'static str {
    if matches!(
        duration.tuplet,
        Some(Tuplet {
            actual: 3,
            normal: 2
        })
    ) {
        "&"
    } else {
        match duration.dots {
            1 => ".",
            2 => "..",
            _ => "",
        }
    }
}
