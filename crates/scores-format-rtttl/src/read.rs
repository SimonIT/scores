use scores_core::{
    Duration, Event, Measure, MeasureAttributes, Note, Part, Pitch, Rest, Result, Score,
    ScoresError, Tie,
};

pub fn read(data: &[u8]) -> Result<Score> {
    let text = std::str::from_utf8(data)
        .map_err(|e| ScoresError::Parse(format!("input is not valid utf-8: {e}")))?;

    let (name, tones) =
        rtttl::parse(text).map_err(|e| ScoresError::Parse(format!("invalid RTTTL: {e}")))?;
    let bpm = parse_bpm(text) as f64;

    let mut events = Vec::new();
    for tone in tones {
        let (frequency, duration) =
            tone.map_err(|e| ScoresError::Parse(format!("invalid RTTTL tone: {e}")))?;
        let quarter_length = duration.as_secs_f64() * bpm / 60.0;
        let notated = Duration::from_quarter_length_quantized(quarter_length);

        events.push(match frequency {
            Some(hz) => {
                let midi_number = (69.0 + 12.0 * (f64::from(hz) / 440.0).log2()).round() as i32;
                Event::Note(Note::new(
                    Pitch::from_midi_number(midi_number, false),
                    notated,
                    Tie::None,
                ))
            }
            None => Event::Rest(Rest::new(notated)),
        });
    }

    let title = name.trim();
    let title = (!title.is_empty()).then(|| title.to_string());

    Ok(Score {
        title,
        composer: None,
        parts: vec![Part {
            id: "P1".to_string(),
            name: None,
            measures: vec![Measure::single_voice(
                1,
                MeasureAttributes::default(),
                events,
            )],
        }],
    })
}

/// `rtttl::parse` resolves BPM into each tone's wall-clock `Duration`
/// without exposing the value itself, so it's re-read directly from the
/// `b=` default parameter (falling back to the crate's own default of 63).
fn parse_bpm(text: &str) -> u16 {
    let Some(defaults) = text.split(':').nth(1) else {
        return 63;
    };
    for part in defaults.split(',') {
        if let Some((key, value)) = part.split_once('=')
            && key.trim().eq_ignore_ascii_case("b")
            && let Ok(bpm) = value.trim().parse()
        {
            return bpm;
        }
    }
    63
}
