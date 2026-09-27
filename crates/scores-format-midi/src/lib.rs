//! Standard MIDI file plugin, built on the [`midly`] crate for the actual
//! byte-level format (chunk parsing, variable-length quantities, running
//! status, ...).
//!
//! MIDI carries no notated structure (no bar lines, ties, or written
//! durations - only ticks), so the read/write modules handle that domain
//! mapping themselves; see their module docs for exactly how. Tempo and
//! time-signature changes are read/written wherever they occur in the file
//! (not just at the start), each attached to whichever measure its tick
//! falls at the start of.
//!
//! Since MIDI is a performance format, notation is mostly *rendered* on
//! writing rather than stored, and only partly recoverable on reading:
//!
//! | Feature | Write | Read |
//! |---|---|---|
//! | Dynamics | note velocity (`dynamics`) | velocity bucketed back into a dynamic |
//! | Wedges (hairpins) | linear velocity ramp | no (reads as successive dynamics) |
//! | Articulations | staccato(issimo)/spiccato/portato shorten, accent/marcato boost velocity | accent/marcato only |
//! | Ornaments | trill, mordents, turns, tremolo realized as notes | no |
//! | Fermata | tempo dip + `Marker` | yes (and the dip is undone) |
//! | Grace notes | short notes stolen from the next event | no |
//! | Chord symbols | `Text` meta event | yes |
//! | Repeats, 1st/2nd endings, D.C./D.S./Coda/Segno/Fine | unrolled into playback order ([`scores_core::playback_order`]) | no (the unrolled measures) |
//! | Barline style | no | no |
//!
//! The details and exact rules live in the `write`, `read` and
//! `performance` module docs and in [`scores_core::playback_order`].

mod dynamics;
mod performance;
mod read;
mod write;

use scores_core::{Format, FormatReader, FormatWriter, Result, Score};

pub struct MidiFormat;

impl FormatReader for MidiFormat {
    fn read(&self, data: &[u8]) -> Result<Score> {
        read::read(data)
    }
}

impl FormatWriter for MidiFormat {
    fn write(&self, score: &Score) -> Result<Vec<u8>> {
        write::write(score)
    }
}

impl Format for MidiFormat {
    fn id(&self) -> &'static str {
        "midi"
    }

    fn name(&self) -> &'static str {
        "Standard MIDI File"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["mid", "midi"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::{
        Articulation, ChordSymbol, Duration, Dynamic, Event, KeySignature, Measure,
        MeasureAttributes, Mode, Navigation, Note, NoteValue, Ornament, Part, Pitch, Rest, Score,
        Step, Tempo, Tie, TimeSignature, Wedge,
    };

    fn scale_score() -> Score {
        let notes = [
            (Step::C, 0, 4),
            (Step::D, 0, 4),
            (Step::E, 0, 4),
            (Step::F, 0, 4),
        ];
        let events = notes
            .iter()
            .map(|&(step, alter, octave)| {
                Event::Note(Note::new(
                    Pitch::new(step, alter, octave),
                    Duration::new(NoteValue::Quarter),
                    Tie::None,
                ))
            })
            .collect();

        Score {
            title: Some("Scale".to_string()),
            composer: None,
            parts: vec![Part {
                id: "P1".to_string(),
                name: Some("Piano".to_string()),
                measures: vec![Measure::single_voice(
                    1,
                    MeasureAttributes {
                        time_signature: Some(TimeSignature {
                            beats: 4,
                            beat_type: 4,
                        }),
                        key_signature: Some(KeySignature {
                            fifths: 0,
                            mode: Mode::Major,
                        }),
                        clef: None,
                        tempo: Some(Tempo { bpm: 120.0 }),
                    },
                    events,
                )],
            }],
        }
    }

    #[test]
    fn writes_a_parseable_midi_file() {
        let bytes = MidiFormat.write(&scale_score()).unwrap();
        assert_eq!(&bytes[0..4], b"MThd");
        midly::Smf::parse(&bytes).expect("midly should parse what we wrote");
    }

    #[test]
    fn round_trips_pitches_and_durations() {
        let original = scale_score();
        let bytes = MidiFormat.write(&original).unwrap();
        let reread = MidiFormat.read(&bytes).unwrap();

        assert_eq!(reread.title, original.title);
        assert_eq!(reread.parts.len(), 1);

        let events = &reread.parts[0].measures[0].voices[0].events;
        assert_eq!(events.len(), 4);
        for (original_event, reread_event) in original.parts[0].measures[0].voices[0]
            .events
            .iter()
            .zip(events)
        {
            let (Event::Note(a), Event::Note(b)) = (original_event, reread_event) else {
                panic!()
            };
            assert_eq!(a.pitch, b.pitch);
            assert_eq!(a.duration.quarter_length(), b.duration.quarter_length());
        }
    }

    #[test]
    fn ties_merge_into_one_sounding_note() {
        let mut score = scale_score();
        score.parts[0].measures[0].voices[0].events = vec![
            Event::Note(Note::new(
                Pitch::new(Step::C, 0, 4),
                Duration::new(NoteValue::Quarter),
                Tie::Start,
            )),
            Event::Note(Note::new(
                Pitch::new(Step::C, 0, 4),
                Duration::new(NoteValue::Quarter),
                Tie::Stop,
            )),
        ];

        let bytes = MidiFormat.write(&score).unwrap();
        let smf = midly::Smf::parse(&bytes).unwrap();
        let part_track = &smf.tracks[1];

        let note_events: Vec<_> = part_track
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    midly::TrackEventKind::Midi {
                        message: midly::MidiMessage::NoteOn { .. }
                            | midly::MidiMessage::NoteOff { .. },
                        ..
                    }
                )
            })
            .collect();
        // One note-on, then one note-off after the combined (half-note) duration - not two separate notes.
        assert_eq!(note_events.len(), 2);
    }

    #[test]
    fn mid_file_tempo_and_time_signature_changes_round_trip() {
        let mut score = scale_score();
        let second_measure = Measure::single_voice(
            2,
            MeasureAttributes {
                time_signature: Some(TimeSignature {
                    beats: 3,
                    beat_type: 4,
                }),
                key_signature: None,
                clef: None,
                tempo: Some(Tempo { bpm: 90.0 }),
            },
            vec![
                Event::Note(Note::new(
                    Pitch::new(Step::G, 0, 4),
                    Duration::new(NoteValue::Quarter),
                    Tie::None,
                )),
                Event::Note(Note::new(
                    Pitch::new(Step::A, 0, 4),
                    Duration::new(NoteValue::Quarter),
                    Tie::None,
                )),
                Event::Note(Note::new(
                    Pitch::new(Step::B, 0, 4),
                    Duration::new(NoteValue::Quarter),
                    Tie::None,
                )),
            ],
        );
        score.parts[0].measures.push(second_measure);

        let bytes = MidiFormat.write(&score).unwrap();
        let reread = MidiFormat.read(&bytes).unwrap();
        let measures = &reread.parts[0].measures;

        assert_eq!(measures.len(), 2);
        // Tempo round-trips through a micros-per-quarter conversion, so
        // compare approximately rather than bit-for-bit.
        assert!((measures[0].attributes.tempo.unwrap().bpm - 120.0).abs() < 0.01);
        assert_eq!(
            measures[0].attributes.time_signature,
            Some(TimeSignature {
                beats: 4,
                beat_type: 4
            })
        );

        assert!((measures[1].attributes.tempo.unwrap().bpm - 90.0).abs() < 0.01);
        assert_eq!(
            measures[1].attributes.time_signature,
            Some(TimeSignature {
                beats: 3,
                beat_type: 4
            })
        );
        assert_eq!(measures[1].voices[0].events.len(), 3);
        let Event::Note(first) = &measures[1].voices[0].events[0] else {
            panic!()
        };
        assert_eq!(first.pitch.step, Step::G);
    }

    #[test]
    fn dynamics_round_trip_through_velocity_buckets() {
        let mut score = scale_score();
        let mut loud = Note::new(
            Pitch::new(Step::G, 0, 4),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        );
        loud.dynamic = Some(Dynamic::Ff);
        score.parts[0].measures[0].voices[0]
            .events
            .push(Event::Note(loud));

        let bytes = MidiFormat.write(&score).unwrap();
        let reread = MidiFormat.read(&bytes).unwrap();
        let events: Vec<&Event> = reread.parts[0]
            .measures
            .iter()
            .flat_map(|m| m.voices.iter().flat_map(|v| v.events.iter()))
            .collect();

        // The first note picks up a dynamic (f is the default-velocity bucket).
        let Event::Note(first) = events[0] else {
            panic!()
        };
        assert_eq!(first.dynamic, Some(Dynamic::F));

        let Event::Note(last) = events.last().unwrap() else {
            panic!()
        };
        assert_eq!(last.dynamic, Some(Dynamic::Ff));
    }

    fn quarter(step: Step) -> Note {
        Note::new(
            Pitch::new(step, 0, 4),
            Duration::new(NoteValue::Quarter),
            Tie::None,
        )
    }

    /// `(on tick, off tick, key, velocity)` of every note in a track.
    fn sounding_notes(track: &[midly::TrackEvent]) -> Vec<(u32, u32, u8, u8)> {
        let mut tick = 0;
        let mut open: Vec<(u32, u8, u8)> = Vec::new();
        let mut notes = Vec::new();
        for event in track {
            tick += u32::from(event.delta);
            match event.kind {
                midly::TrackEventKind::Midi {
                    message: midly::MidiMessage::NoteOn { key, vel },
                    ..
                } if vel > 0 => open.push((tick, key.as_int(), vel.as_int())),
                midly::TrackEventKind::Midi {
                    message: midly::MidiMessage::NoteOff { key, .. },
                    ..
                } => {
                    let index = open.iter().position(|o| o.1 == key.as_int()).unwrap();
                    let (on, key, vel) = open.remove(index);
                    notes.push((on, tick, key, vel));
                }
                _ => {}
            }
        }
        notes.sort();
        notes
    }

    fn written_notes(score: &Score) -> Vec<(u32, u32, u8, u8)> {
        let bytes = MidiFormat.write(score).unwrap();
        let smf = midly::Smf::parse(&bytes).unwrap();
        sounding_notes(&smf.tracks[1])
    }

    fn all_events(score: &Score) -> Vec<Event> {
        score.parts[0]
            .measures
            .iter()
            .flat_map(|m| m.voices[0].events.clone())
            .collect()
    }

    #[test]
    fn repeats_are_unrolled() {
        let mut score = scale_score();
        let mut second = score.parts[0].measures[0].clone();
        second.number = 2;
        second.attributes = MeasureAttributes::default();
        score.parts[0].measures[0].barline.repeat_end = Some(2);
        second.navigation = vec![Navigation::Fine];
        score.parts[0].measures.push(second);

        let reread = MidiFormat.read(&MidiFormat.write(&score).unwrap()).unwrap();
        // The repeated first measure is played twice, then the second.
        assert_eq!(reread.parts[0].measures.len(), 3);
        // Structure is realized, not re-read as structure.
        assert!(
            reread.parts[0]
                .measures
                .iter()
                .all(|m| m.navigation.is_empty() && m.barline.repeat_end.is_none())
        );
    }

    #[test]
    fn da_capo_al_fine_is_followed() {
        let mut score = scale_score();
        let measure = score.parts[0].measures[0].clone();
        score.parts[0].measures[0].navigation = vec![Navigation::Fine];
        let mut second = measure.clone();
        second.attributes = MeasureAttributes::default();
        second.navigation = vec![Navigation::DaCapoAlFine];
        score.parts[0].measures.push(second);
        // Measure 1, measure 2, back to measure 1 and stop at Fine.
        assert_eq!(written_notes(&score).len(), 12);
    }

    #[test]
    fn staccato_shortens_without_moving_later_notes() {
        let mut score = scale_score();
        let Event::Note(first) = &mut score.parts[0].measures[0].voices[0].events[0] else {
            panic!()
        };
        first.articulations.push(Articulation::Staccato);
        let notes = written_notes(&score);
        assert_eq!((notes[0].0, notes[0].1), (0, 240));
        assert_eq!(notes[1].0, 480);
    }

    #[test]
    fn accent_round_trips_without_a_dynamic_change() {
        let mut score = scale_score();
        let Event::Note(second) = &mut score.parts[0].measures[0].voices[0].events[1] else {
            panic!()
        };
        second.articulations.push(Articulation::Accent);
        let notes = written_notes(&score);
        assert_eq!(notes[1].3, notes[0].3 + 20);

        let reread = MidiFormat.read(&MidiFormat.write(&score).unwrap()).unwrap();
        let events = all_events(&reread);
        let Event::Note(accented) = &events[1] else {
            panic!()
        };
        assert_eq!(accented.articulations, vec![Articulation::Accent]);
        assert_eq!(accented.dynamic, None);
        let Event::Note(after) = &events[2] else {
            panic!()
        };
        assert_eq!(after.dynamic, None);
    }

    #[test]
    fn fermata_dips_tempo_and_round_trips() {
        let mut score = scale_score();
        let Event::Note(second) = &mut score.parts[0].measures[0].voices[0].events[1] else {
            panic!()
        };
        second.fermata = true;

        let bytes = MidiFormat.write(&score).unwrap();
        let smf = midly::Smf::parse(&bytes).unwrap();
        let mut tick = 0;
        let mut tempos = Vec::new();
        for event in &smf.tracks[0] {
            tick += u32::from(event.delta);
            if let midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(micros)) = event.kind {
                tempos.push((tick, micros.as_int()));
            }
        }
        assert_eq!(tempos, vec![(0, 500_000), (480, 1_000_000), (960, 500_000)]);
        // The note timeline itself is untouched.
        assert_eq!(sounding_notes(&smf.tracks[1])[2].0, 960);

        let reread = MidiFormat.read(&bytes).unwrap();
        let events = all_events(&reread);
        let Event::Note(held) = &events[1] else {
            panic!()
        };
        assert!(held.fermata);
        let measure = &reread.parts[0].measures[0];
        assert!((measure.attributes.tempo.unwrap().bpm - 120.0).abs() < 0.01);
    }

    #[test]
    fn grace_notes_steal_time_from_the_following_note() {
        let mut score = scale_score();
        let mut grace = Note::new(
            Pitch::new(Step::B, 0, 3),
            Duration::new(NoteValue::Eighth),
            Tie::None,
        );
        grace.grace = true;
        score.parts[0].measures[0].voices[0]
            .events
            .insert(1, Event::Note(grace));

        let notes = written_notes(&score);
        assert_eq!(notes.len(), 5);
        // The grace plays at the principal's beat, the principal is delayed
        // but still ends on time, and later notes aren't shifted.
        assert_eq!((notes[1].0, notes[1].1, notes[1].2), (480, 540, 59));
        assert_eq!((notes[2].0, notes[2].1), (540, 960));
        assert_eq!(notes[3].0, 960);
    }

    #[test]
    fn chord_symbols_round_trip_as_text_events() {
        let mut score = scale_score();
        let events = &mut score.parts[0].measures[0].voices[0].events;
        events[0].set_chord_symbol(ChordSymbol::parse("Cmaj7"));
        events[2] = Event::Rest(Rest::new(Duration::new(NoteValue::Quarter)));
        events[2].set_chord_symbol(ChordSymbol::parse("F#m7b5/E"));

        let reread = MidiFormat.read(&MidiFormat.write(&score).unwrap()).unwrap();
        let events = all_events(&reread);
        assert_eq!(
            events[0].chord_symbol(),
            ChordSymbol::parse("Cmaj7").as_ref()
        );
        assert_eq!(events[1].chord_symbol(), None);
        assert!(matches!(events[2], Event::Rest(_)));
        assert_eq!(
            events[2].chord_symbol(),
            ChordSymbol::parse("F#m7b5/E").as_ref()
        );
    }

    #[test]
    fn crescendo_ramps_velocity_to_the_target_dynamic() {
        let mut score = scale_score();
        let events = &mut score.parts[0].measures[0].voices[0].events;
        let mut start = quarter(Step::C);
        start.dynamic = Some(Dynamic::P);
        start.wedge = Some(Wedge::CrescendoStart);
        events[0] = Event::Note(start);
        let mut stop = quarter(Step::F);
        stop.wedge = Some(Wedge::Stop);
        stop.dynamic = Some(Dynamic::F);
        events[3] = Event::Note(stop);

        let velocities: Vec<u8> = written_notes(&score).iter().map(|n| n.3).collect();
        assert_eq!(velocities, vec![40, 56, 72, 88]);
    }

    #[test]
    fn trill_is_realized_as_alternating_notes() {
        let mut score = scale_score();
        let Event::Note(first) = &mut score.parts[0].measures[0].voices[0].events[0] else {
            panic!()
        };
        first.ornaments.push(Ornament::TrillMark);
        let notes = written_notes(&score);
        let trill: Vec<u8> = notes
            .iter()
            .take_while(|n| n.0 < 480)
            .map(|n| n.2)
            .collect();
        assert_eq!(trill, vec![60, 62, 60, 62, 60, 62, 60, 62]);
        assert_eq!(notes[8].0, 480);
    }
}
