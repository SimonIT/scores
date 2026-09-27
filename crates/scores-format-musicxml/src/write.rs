//! Builds a `musicxml::elements::ScorePartwise` tree from a [`Score`] and
//! serializes it with the crate's own writer, rather than formatting XML by
//! hand. See the module docs in [`crate`] for what is (and isn't) mapped.

use musicxml::datatypes as dt;
use musicxml::elements as xml;
use scores_core::{
    Articulation, Barline, BarlineType, ChordKind, ChordRoot, ChordSymbol, Clef, Duration, Dynamic,
    Ending, Event, Measure, MeasureAttributes, Mode, Navigation, Note, NoteValue, Ornament, Part,
    Rest, Result, Score, ScoresError, Step, Tempo, Tie, Wedge,
};

/// Divisions (ticks) per quarter note. Chosen so that every base note value
/// (down to a 64th) with up to two augmentation dots, and any tuplet ratio
/// whose denominator divides it, converts to an exact integer.
const DIVISIONS: u32 = 960;

pub fn write(score: &Score) -> Result<Vec<u8>> {
    let xml_score = build_score(score);
    musicxml::write_partwise_score_data(&xml_score, false, false)
        .map_err(|e| ScoresError::Write(format!("failed to serialize MusicXML: {e}")))
}

fn build_score(score: &Score) -> xml::ScorePartwise {
    let part_list = xml::PartList {
        attributes: (),
        content: xml::PartListContents {
            content: score
                .parts
                .iter()
                .map(|part| xml::PartListElement::ScorePart(build_score_part(part)))
                .collect(),
        },
    };

    let identification = score.composer.as_ref().map(|composer| xml::Identification {
        attributes: (),
        content: xml::IdentificationContents {
            creator: vec![xml::Creator {
                attributes: xml::CreatorAttributes {
                    r#type: Some(dt::Token("composer".to_string())),
                },
                content: composer.clone(),
            }],
            ..Default::default()
        },
    });

    xml::ScorePartwise {
        attributes: xml::ScorePartwiseAttributes {
            version: Some(dt::Token("4.0".to_string())),
        },
        content: xml::ScorePartwiseContents {
            work: None,
            movement_number: None,
            movement_title: score.title.clone().map(|title| xml::MovementTitle {
                attributes: (),
                content: title,
            }),
            identification,
            defaults: None,
            credit: Vec::new(),
            part_list,
            part: score.parts.iter().map(build_part).collect(),
        },
    }
}

fn build_score_part(part: &Part) -> xml::ScorePart {
    xml::ScorePart {
        attributes: xml::ScorePartAttributes {
            id: dt::Id(part.id.clone()),
        },
        content: xml::ScorePartContents {
            identification: None,
            part_link: Vec::new(),
            part_name: xml::PartName {
                attributes: xml::PartNameAttributes::default(),
                content: part.name.clone().unwrap_or_else(|| part.id.clone()),
            },
            part_name_display: None,
            part_abbreviation: None,
            part_abbreviation_display: None,
            group: Vec::new(),
            score_instrument: Vec::new(),
            player: Vec::new(),
            midi_device: Vec::new(),
            midi_instrument: Vec::new(),
        },
    }
}

fn build_part(part: &Part) -> xml::Part {
    xml::Part {
        attributes: xml::PartAttributes {
            id: dt::IdRef(part.id.clone()),
        },
        content: {
            let stops = ending_stops(&part.measures);
            part.measures
                .iter()
                .zip(stops)
                .enumerate()
                .map(|(index, (measure, stop))| {
                    xml::PartElement::Measure(build_measure(measure, index, stop))
                })
                .collect()
        },
    }
}

/// For each measure, the ending bracket (if any) whose last measure it is,
/// i.e. that gets its `stop`/`discontinue` on this measure's right barline.
/// A bracket's span is clamped to the part.
fn ending_stops(measures: &[Measure]) -> Vec<Option<&Ending>> {
    let mut stops = vec![None; measures.len()];
    for (start, measure) in measures.iter().enumerate() {
        if let Some(ending) = &measure.barline.ending {
            let last = (start + ending.measures.max(1) as usize - 1).min(measures.len() - 1);
            stops[last] = Some(ending);
        }
    }
    stops
}

/// Multiple voices share one flat MusicXML timeline: after each voice but
/// the last, a `<backup>` rewinds by that voice's total sounding duration
/// so the next voice starts back at the beginning of the measure.
///
/// Segno/Coda marks go at the start of the measure; the jump instructions
/// (D.C./D.S./Fine/To Coda) at its end, where they take effect.
fn build_measure(measure: &Measure, index: usize, ending_stop: Option<&Ending>) -> xml::Measure {
    let mut content = Vec::new();
    let attrs = &measure.attributes;

    if let Some(left) = build_left_barline(&measure.barline) {
        content.push(xml::MeasureElement::Barline(left));
    }
    if index == 0
        || attrs.time_signature.is_some()
        || attrs.key_signature.is_some()
        || attrs.clef.is_some()
    {
        content.push(xml::MeasureElement::Attributes(build_attributes(
            attrs,
            index == 0,
        )));
    }
    if let Some(tempo) = attrs.tempo {
        content.push(xml::MeasureElement::Sound(build_tempo_sound(tempo)));
    }
    let at_start = |n: &&Navigation| matches!(n, Navigation::Segno | Navigation::Coda);
    for &navigation in measure.navigation.iter().filter(at_start) {
        content.push(xml::MeasureElement::Direction(build_navigation_direction(
            navigation,
        )));
    }

    for (voice_index, voice) in measure.voices.iter().enumerate() {
        if voice_index > 0 {
            let previous = &measure.voices[voice_index - 1];
            let rewind: u32 = previous.events.iter().map(event_divisions).sum();
            content.push(xml::MeasureElement::Backup(build_backup(rewind)));
        }
        for event in &voice.events {
            push_event(event, voice.number, &mut content);
        }
    }

    for &navigation in measure.navigation.iter().filter(|n| !at_start(n)) {
        content.push(xml::MeasureElement::Direction(build_navigation_direction(
            navigation,
        )));
    }
    if let Some(right) = build_right_barline(&measure.barline, ending_stop) {
        content.push(xml::MeasureElement::Barline(right));
    }

    xml::Measure {
        attributes: xml::MeasureAttributes {
            number: dt::Token((measure.number.max(1)).to_string()),
            id: None,
            implicit: None,
            non_controlling: None,
            text: None,
            width: None,
        },
        content,
    }
}

/// A grace note's declared duration is nominal (it borrows no real time),
/// so it contributes nothing to how far a `<backup>` needs to rewind.
fn event_divisions(event: &Event) -> u32 {
    match event {
        Event::Rest(rest) => sounding_divisions(&rest.duration),
        Event::Note(note) => event_divisions_of_note(note),
        Event::Chord(notes) => notes.first().map(event_divisions_of_note).unwrap_or(0),
    }
}

fn event_divisions_of_note(note: &Note) -> u32 {
    if note.grace {
        0
    } else {
        sounding_divisions(&note.duration)
    }
}

fn push_event(event: &Event, voice_number: u8, content: &mut Vec<xml::MeasureElement>) {
    // `<harmony>` precedes the note/rest it sits over.
    if let Some(symbol) = event.chord_symbol() {
        content.push(xml::MeasureElement::Harmony(build_harmony(symbol)));
    }
    match event {
        Event::Note(note) => {
            if let Some(direction) = build_marking_direction(note.dynamic.as_ref(), note.wedge) {
                content.push(xml::MeasureElement::Direction(direction));
            }
            content.push(xml::MeasureElement::Note(build_note(
                note,
                false,
                voice_number,
            )));
        }
        Event::Rest(rest) => {
            content.push(xml::MeasureElement::Note(build_rest(rest, voice_number)))
        }
        Event::Chord(notes) => {
            for (i, note) in notes.iter().enumerate() {
                if i == 0
                    && let Some(direction) =
                        build_marking_direction(note.dynamic.as_ref(), note.wedge)
                {
                    content.push(xml::MeasureElement::Direction(direction));
                }
                content.push(xml::MeasureElement::Note(build_note(
                    note,
                    i > 0,
                    voice_number,
                )));
            }
        }
    }
}

fn build_marking_direction(
    dynamic: Option<&Dynamic>,
    wedge: Option<Wedge>,
) -> Option<xml::Direction> {
    let mut direction_types = Vec::new();
    if let Some(dynamic) = dynamic
        && let Some(dynamics_type) = dynamic_to_xml(dynamic)
    {
        direction_types.push(xml::DirectionType {
            attributes: xml::DirectionTypeAttributes::default(),
            content: xml::DirectionTypeContents::Dynamics(vec![xml::Dynamics {
                attributes: xml::DynamicsAttributes::default(),
                content: vec![dynamics_type],
            }]),
        });
    }
    if let Some(wedge) = wedge {
        direction_types.push(xml::DirectionType {
            attributes: xml::DirectionTypeAttributes::default(),
            content: xml::DirectionTypeContents::Wedge(wedge_to_xml(wedge)),
        });
    }
    if direction_types.is_empty() {
        return None;
    }
    Some(xml::Direction {
        attributes: xml::DirectionAttributes::default(),
        content: xml::DirectionContents {
            direction_type: direction_types,
            ..Default::default()
        },
    })
}

/// A chord symbol as a single `<harmony>` harmony-chord. "N.C." is kind
/// `none`; it still gets a (meaningless) root, since the `musicxml` crate
/// only recognizes a harmony-chord that has one. [`ChordKind::Other`] is
/// kind `other` with its suffix as the display `text`.
fn build_harmony(symbol: &ChordSymbol) -> xml::Harmony {
    let (kind, text) = chord_kind_to_xml(&symbol.kind);
    xml::Harmony {
        attributes: xml::HarmonyAttributes::default(),
        content: xml::HarmonyContents {
            harmony: vec![xml::HarmonySubcontents {
                root: Some(xml::Root {
                    attributes: (),
                    content: xml::RootContents {
                        root_step: xml::RootStep {
                            attributes: xml::RootStepAttributes::default(),
                            content: to_xml_step(symbol.root.step),
                        },
                        root_alter: (symbol.root.alter != 0).then(|| xml::RootAlter {
                            attributes: xml::RootAlterAttributes::default(),
                            content: dt::Semitones(symbol.root.alter as i16),
                        }),
                    },
                }),
                numeral: None,
                function: None,
                kind: xml::Kind {
                    attributes: xml::KindAttributes {
                        text: text.map(|t| dt::Token(t.to_string())),
                        ..Default::default()
                    },
                    content: kind,
                },
                inversion: None,
                bass: symbol
                    .bass
                    .filter(|_| symbol.kind != ChordKind::NoChord)
                    .map(build_bass),
                degree: Vec::new(),
            }],
            ..Default::default()
        },
    }
}

fn build_bass(bass: ChordRoot) -> xml::Bass {
    xml::Bass {
        attributes: xml::BassAttributes::default(),
        content: xml::BassContents {
            bass_separator: None,
            bass_step: xml::BassStep {
                attributes: xml::BassStepAttributes::default(),
                content: to_xml_step(bass.step),
            },
            bass_alter: (bass.alter != 0).then(|| xml::BassAlter {
                attributes: xml::BassAlterAttributes::default(),
                content: dt::Semitones(bass.alter as i16),
            }),
        },
    }
}

/// The `kind-value` for a [`ChordKind`], plus the display `text` for the
/// kinds that need one.
fn chord_kind_to_xml(kind: &ChordKind) -> (dt::KindValue, Option<&str>) {
    use dt::KindValue as K;
    let value = match kind {
        ChordKind::Major => K::Major,
        ChordKind::Minor => K::Minor,
        ChordKind::Augmented => K::Augmented,
        ChordKind::Diminished => K::Diminished,
        ChordKind::Dominant => K::Dominant,
        ChordKind::MajorSeventh => K::MajorSeventh,
        ChordKind::MinorSeventh => K::MinorSeventh,
        ChordKind::DiminishedSeventh => K::DiminishedSeventh,
        ChordKind::AugmentedSeventh => K::AugmentedSeventh,
        ChordKind::HalfDiminished => K::HalfDiminished,
        ChordKind::MajorMinor => K::MajorMinor,
        ChordKind::MajorSixth => K::MajorSixth,
        ChordKind::MinorSixth => K::MinorSixth,
        ChordKind::DominantNinth => K::DominantNinth,
        ChordKind::MajorNinth => K::MajorNinth,
        ChordKind::MinorNinth => K::MinorNinth,
        ChordKind::Dominant11th => K::Dominant11th,
        ChordKind::Major11th => K::Major11th,
        ChordKind::Minor11th => K::Minor11th,
        ChordKind::Dominant13th => K::Dominant13th,
        ChordKind::Major13th => K::Major13th,
        ChordKind::Minor13th => K::Minor13th,
        ChordKind::SuspendedSecond => K::SuspendedSecond,
        ChordKind::SuspendedFourth => K::SuspendedFourth,
        ChordKind::Power => K::Power,
        ChordKind::NoChord => return (K::None, Some("N.C.")),
        ChordKind::Other(suffix) => return (K::Other, Some(suffix)),
    };
    (value, None)
}

fn build_tempo_sound(tempo: Tempo) -> xml::Sound {
    xml::Sound {
        attributes: sound_attributes_with_tempo(tempo.bpm as f64),
        content: xml::SoundContents::default(),
    }
}

/// A navigation mark is written twice over, as is conventional: a visible
/// `<segno>`/`<coda>` symbol or `<words>` text for notation programs, plus
/// a `<sound>` with the matching playback attributes.
fn build_navigation_direction(navigation: Navigation) -> xml::Direction {
    let direction_type = match navigation {
        Navigation::Segno => xml::DirectionTypeContents::Segno(vec![xml::Segno {
            attributes: xml::SegnoAttributes::default(),
            content: (),
        }]),
        Navigation::Coda => xml::DirectionTypeContents::Coda(vec![xml::Coda {
            attributes: xml::CodaAttributes::default(),
            content: (),
        }]),
        other => xml::DirectionTypeContents::Words(vec![xml::Words {
            attributes: xml::WordsAttributes::default(),
            content: navigation_text(other).to_string(),
        }]),
    };
    xml::Direction {
        attributes: xml::DirectionAttributes {
            placement: Some(dt::AboveBelow::Above),
            ..Default::default()
        },
        content: xml::DirectionContents {
            direction_type: vec![xml::DirectionType {
                attributes: xml::DirectionTypeAttributes::default(),
                content: direction_type,
            }],
            sound: build_navigation_sound(navigation),
            ..Default::default()
        },
    }
}

fn navigation_text(navigation: Navigation) -> &'static str {
    match navigation {
        Navigation::Segno => "Segno",
        Navigation::Coda => "Coda",
        Navigation::DaCapo => "D.C.",
        Navigation::DaCapoAlFine => "D.C. al Fine",
        Navigation::DaCapoAlCoda => "D.C. al Coda",
        Navigation::DalSegno => "D.S.",
        Navigation::DalSegnoAlFine => "D.S. al Fine",
        Navigation::DalSegnoAlCoda => "D.S. al Coda",
        Navigation::Fine => "Fine",
        Navigation::ToCoda => "To Coda",
    }
}

fn build_navigation_sound(navigation: Navigation) -> Option<xml::Sound> {
    use scores_core::Navigation as N;
    let mut attrs = empty_sound_attributes();
    match navigation {
        N::DaCapo => attrs.dacapo = Some(dt::YesNo::Yes),
        N::DaCapoAlFine => {
            attrs.dacapo = Some(dt::YesNo::Yes);
            attrs.fine = Some(dt::Token("yes".to_string()));
        }
        N::DaCapoAlCoda => {
            attrs.dacapo = Some(dt::YesNo::Yes);
            attrs.tocoda = Some(dt::Token("coda1".to_string()));
        }
        N::DalSegno => attrs.dalsegno = Some(dt::Token("segno1".to_string())),
        N::DalSegnoAlFine => {
            attrs.dalsegno = Some(dt::Token("segno1".to_string()));
            attrs.fine = Some(dt::Token("yes".to_string()));
        }
        N::DalSegnoAlCoda => {
            attrs.dalsegno = Some(dt::Token("segno1".to_string()));
            attrs.tocoda = Some(dt::Token("coda1".to_string()));
        }
        N::Fine => attrs.fine = Some(dt::Token("yes".to_string())),
        N::ToCoda => attrs.tocoda = Some(dt::Token("coda1".to_string())),
        N::Segno => attrs.segno = Some(dt::Token("segno1".to_string())),
        N::Coda => attrs.coda = Some(dt::Token("coda1".to_string())),
    }
    Some(xml::Sound {
        attributes: attrs,
        content: xml::SoundContents::default(),
    })
}

fn sound_attributes_with_tempo(bpm: f64) -> xml::SoundAttributes {
    let mut attrs = empty_sound_attributes();
    attrs.tempo = Some(dt::NonNegativeDecimal(bpm.max(0.0)));
    attrs
}

/// `SoundAttributes` has ~20 cosmetic/playback fields and doesn't derive
/// `Default` (despite all of them being optional), so this fills them in
/// once.
fn empty_sound_attributes() -> xml::SoundAttributes {
    xml::SoundAttributes {
        coda: None,
        dacapo: None,
        dalsegno: None,
        damper_pedal: None,
        divisions: None,
        dynamics: None,
        elevation: None,
        fine: None,
        forward_repeat: None,
        id: None,
        pan: None,
        pizzicato: None,
        segno: None,
        soft_pedal: None,
        sostenuto_pedal: None,
        tempo: None,
        time_only: None,
        tocoda: None,
    }
}

fn build_backup(divisions: u32) -> xml::Backup {
    xml::Backup {
        attributes: (),
        content: xml::BackupContents {
            duration: xml::Duration {
                attributes: (),
                content: dt::PositiveDivisions(divisions),
            },
            footnote: None,
            level: None,
        },
    }
}

/// The left barline carries a forward repeat and/or the start of an ending
/// bracket (MusicXML starts endings on the left and stops them on the right).
fn build_left_barline(barline: &Barline) -> Option<xml::Barline> {
    let repeat = barline.repeat_start.then_some(xml::Repeat {
        attributes: xml::RepeatAttributes {
            direction: dt::BackwardForward::Forward,
            after_jump: None,
            times: None,
            winged: None,
        },
        content: (),
    });
    let ending = barline
        .ending
        .as_ref()
        .map(|e| ending_to_xml(&e.numbers, dt::StartStopDiscontinue::Start));
    if repeat.is_none() && ending.is_none() {
        return None;
    }
    Some(xml::Barline {
        attributes: xml::BarlineAttributes {
            location: Some(dt::RightLeftMiddle::Left),
            ..Default::default()
        },
        content: xml::BarlineContents {
            repeat,
            ending,
            ..Default::default()
        },
    })
}

/// `ending_stop` is the bracket ending on this measure (see
/// [`ending_stops`]): `stop` (closed hook) or `discontinue` (open).
fn build_right_barline(barline: &Barline, ending_stop: Option<&Ending>) -> Option<xml::Barline> {
    let bar_style = (barline.kind != BarlineType::Regular).then(|| xml::BarStyle {
        attributes: xml::BarStyleAttributes::default(),
        content: barline_type_to_xml(barline.kind),
    });
    let repeat = barline.repeat_end.map(|times| xml::Repeat {
        attributes: xml::RepeatAttributes {
            direction: dt::BackwardForward::Backward,
            after_jump: None,
            times: Some(dt::NonNegativeInteger(times)),
            winged: None,
        },
        content: (),
    });
    let ending = ending_stop.map(|e| {
        let kind = if e.closed {
            dt::StartStopDiscontinue::Stop
        } else {
            dt::StartStopDiscontinue::Discontinue
        };
        ending_to_xml(&e.numbers, kind)
    });

    if bar_style.is_none() && repeat.is_none() && ending.is_none() {
        return None;
    }
    Some(xml::Barline {
        attributes: xml::BarlineAttributes {
            location: Some(dt::RightLeftMiddle::Right),
            ..Default::default()
        },
        content: xml::BarlineContents {
            bar_style,
            repeat,
            ending,
            ..Default::default()
        },
    })
}

/// The element's text is what gets printed on the bracket (`1.`, `1, 2.`);
/// only a start needs it, but it is harmless on the stop.
fn ending_to_xml(numbers: &[u8], kind: dt::StartStopDiscontinue) -> xml::Ending {
    let numbers: Vec<String> = numbers.iter().map(u8::to_string).collect();
    let text = if kind == dt::StartStopDiscontinue::Start && !numbers.is_empty() {
        format!("{}.", numbers.join(", "))
    } else {
        String::new()
    };
    xml::Ending {
        attributes: xml::EndingAttributes {
            number: dt::EndingNumber(numbers.join(",")),
            r#type: kind,
            color: None,
            default_x: None,
            default_y: None,
            end_length: None,
            font_family: None,
            font_size: None,
            font_style: None,
            font_weight: None,
            print_object: None,
            relative_x: None,
            relative_y: None,
            system: None,
            text_x: None,
            text_y: None,
        },
        content: text,
    }
}

fn barline_type_to_xml(kind: BarlineType) -> dt::BarStyle {
    match kind {
        BarlineType::Regular => dt::BarStyle::Regular,
        BarlineType::Dotted => dt::BarStyle::Dotted,
        BarlineType::Dashed => dt::BarStyle::Dashed,
        BarlineType::Heavy => dt::BarStyle::Heavy,
        BarlineType::LightLight => dt::BarStyle::LightLight,
        BarlineType::LightHeavy => dt::BarStyle::LightHeavy,
        BarlineType::HeavyLight => dt::BarStyle::HeavyLight,
        BarlineType::HeavyHeavy => dt::BarStyle::HeavyHeavy,
        BarlineType::Tick => dt::BarStyle::Tick,
        BarlineType::Short => dt::BarStyle::Short,
        BarlineType::NoBarline => dt::BarStyle::None,
    }
}

fn build_attributes(attrs: &MeasureAttributes, include_divisions: bool) -> xml::Attributes {
    let mut content = xml::AttributesContents::default();
    if include_divisions {
        content.divisions = Some(xml::Divisions {
            attributes: (),
            content: dt::PositiveDivisions(DIVISIONS),
        });
    }
    if let Some(key) = attrs.key_signature {
        content.key = vec![xml::Key {
            attributes: xml::KeyAttributes::default(),
            content: xml::KeyContents::Explicit(xml::ExplicitKeyContents {
                cancel: None,
                fifths: xml::Fifths {
                    attributes: (),
                    content: dt::Fifths(key.fifths),
                },
                mode: Some(xml::Mode {
                    attributes: (),
                    content: to_xml_mode(key.mode),
                }),
                key_octave: Vec::new(),
            }),
        }];
    }
    if let Some(ts) = attrs.time_signature {
        content.time = vec![xml::Time {
            attributes: xml::TimeAttributes::default(),
            content: xml::TimeContents {
                beats: vec![xml::TimeBeatContents {
                    beats: xml::Beats {
                        attributes: (),
                        content: ts.beats.to_string(),
                    },
                    beat_type: xml::BeatType {
                        attributes: (),
                        content: ts.beat_type.to_string(),
                    },
                }],
                ..Default::default()
            },
        }];
    }
    if let Some(clef) = attrs.clef {
        content.clef = vec![build_clef(clef)];
    }
    xml::Attributes {
        attributes: (),
        content,
    }
}

fn build_clef(clef: Clef) -> xml::Clef {
    let (sign, line) = match clef {
        Clef::Treble => (dt::ClefSign::G, 2),
        Clef::Bass => (dt::ClefSign::F, 4),
        Clef::Alto => (dt::ClefSign::C, 3),
        Clef::Tenor => (dt::ClefSign::C, 4),
        Clef::Percussion => (dt::ClefSign::Percussion, 2),
    };
    xml::Clef {
        attributes: xml::ClefAttributes::default(),
        content: xml::ClefContents {
            sign: xml::Sign {
                attributes: (),
                content: sign,
            },
            line: Some(xml::Line {
                attributes: (),
                content: dt::StaffLinePosition(line),
            }),
            clef_octave_change: None,
        },
    }
}

fn build_note(note: &Note, is_chord: bool, voice_number: u8) -> xml::Note {
    let tie = xml_ties(note.tie);
    if note.grace {
        build_grace_note(note, is_chord, voice_number, tie)
    } else {
        build_normal_note(
            xml::AudibleType::Pitch(xml_pitch(&note.pitch)),
            &note.duration,
            tie,
            is_chord,
            voice_number,
            note.fermata,
            &note.articulations,
            &note.ornaments,
        )
    }
}

fn build_rest(rest: &Rest, voice_number: u8) -> xml::Note {
    let audible = xml::AudibleType::Rest(xml::Rest {
        attributes: xml::RestAttributes::default(),
        content: xml::RestContents {
            display_step: None,
            display_octave: None,
        },
    });
    build_normal_note(
        audible,
        &rest.duration,
        Vec::new(),
        false,
        voice_number,
        rest.fermata,
        &[],
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
fn build_normal_note(
    audible: xml::AudibleType,
    duration: &Duration,
    tie: Vec<xml::Tie>,
    is_chord: bool,
    voice_number: u8,
    fermata: bool,
    articulations: &[Articulation],
    ornaments: &[Ornament],
) -> xml::Note {
    let info = xml::NoteType::Normal(xml::NormalInfo {
        chord: is_chord.then_some(xml::Chord {
            attributes: (),
            content: (),
        }),
        audible,
        duration: xml::Duration {
            attributes: (),
            content: dt::PositiveDivisions(sounding_divisions(duration)),
        },
        tie,
    });

    let mut content = empty_note_contents(info);
    content.voice = Some(xml::Voice {
        attributes: (),
        content: voice_number.to_string(),
    });
    content.r#type = Some(xml::Type {
        attributes: xml::TypeAttributes::default(),
        content: to_note_type_value(duration.base),
    });
    content.dot = (0..duration.dots)
        .map(|_| xml::Dot {
            attributes: xml::DotAttributes::default(),
            content: (),
        })
        .collect();
    content.notations = build_notations(fermata, articulations, ornaments);
    if let Some(tuplet) = duration.tuplet {
        content.time_modification = Some(xml::TimeModification {
            attributes: (),
            content: xml::TimeModificationContents {
                actual_notes: xml::ActualNotes {
                    attributes: (),
                    content: dt::NonNegativeInteger(tuplet.actual as u32),
                },
                normal_notes: xml::NormalNotes {
                    attributes: (),
                    content: dt::NonNegativeInteger(tuplet.normal as u32),
                },
                normal_type: None,
                normal_dot: Vec::new(),
            },
        });
    }

    xml::Note {
        attributes: xml::NoteAttributes::default(),
        content,
    }
}

/// Grace notes have no `<duration>` element at all (matching the spec -
/// they borrow time visually rather than occupying real, measured time).
fn build_grace_note(
    note: &Note,
    is_chord: bool,
    voice_number: u8,
    tie: Vec<xml::Tie>,
) -> xml::Note {
    let info = xml::NoteType::Grace(xml::GraceInfo {
        grace: xml::Grace {
            attributes: xml::GraceAttributes::default(),
            content: (),
        },
        info: xml::GraceType::Normal(xml::GraceNormalInfo {
            chord: is_chord.then_some(xml::Chord {
                attributes: (),
                content: (),
            }),
            audible: xml::AudibleType::Pitch(xml_pitch(&note.pitch)),
            tie,
        }),
    });

    let mut content = empty_note_contents(info);
    content.voice = Some(xml::Voice {
        attributes: (),
        content: voice_number.to_string(),
    });
    content.r#type = Some(xml::Type {
        attributes: xml::TypeAttributes::default(),
        content: to_note_type_value(note.duration.base),
    });
    content.dot = (0..note.duration.dots)
        .map(|_| xml::Dot {
            attributes: xml::DotAttributes::default(),
            content: (),
        })
        .collect();
    content.notations = build_notations(note.fermata, &note.articulations, &note.ornaments);
    xml::Note {
        attributes: xml::NoteAttributes::default(),
        content,
    }
}

fn build_notations(
    fermata: bool,
    articulations: &[Articulation],
    ornaments: &[Ornament],
) -> Vec<xml::Notations> {
    let mut items = Vec::new();
    let articulation_types: Vec<xml::ArticulationsType> = articulations
        .iter()
        .filter_map(articulation_to_xml)
        .collect();
    if !articulation_types.is_empty() {
        items.push(xml::NotationContentTypes::Articulations(
            xml::Articulations {
                attributes: xml::ArticulationsAttributes::default(),
                content: articulation_types,
            },
        ));
    }
    let ornament_types = ornaments_to_xml(ornaments);
    if !ornament_types.is_empty() {
        items.push(xml::NotationContentTypes::Ornaments(xml::Ornaments {
            attributes: xml::OrnamentsAttributes::default(),
            content: xml::OrnamentContents {
                ornaments: ornament_types,
                accidental_mark: Vec::new(),
            },
        }));
    }
    if fermata {
        items.push(xml::NotationContentTypes::Fermata(xml::Fermata {
            attributes: xml::FermataAttributes::default(),
            content: dt::FermataShape::Normal,
        }));
    }
    if items.is_empty() {
        Vec::new()
    } else {
        vec![xml::Notations {
            attributes: xml::NotationsAttributes::default(),
            content: xml::NotationsContents {
                footnote: None,
                level: None,
                notations: items,
            },
        }]
    }
}

fn articulation_to_xml(articulation: &Articulation) -> Option<xml::ArticulationsType> {
    use xml::ArticulationsType as T;
    Some(match articulation {
        Articulation::Accent => T::Accent(xml::Accent {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::StrongAccent => T::StrongAccent(xml::StrongAccent {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Staccato => T::Staccato(xml::Staccato {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Tenuto => T::Tenuto(xml::Tenuto {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::DetachedLegato => T::DetachedLegato(xml::DetachedLegato {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Staccatissimo => T::Staccatissimo(xml::Staccatissimo {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Spiccato => T::Spiccato(xml::Spiccato {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Scoop => T::Scoop(xml::Scoop {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Plop => T::Plop(xml::Plop {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Doit => T::Doit(xml::Doit {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Falloff => T::Falloff(xml::Falloff {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Stress => T::Stress(xml::Stress {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::Unstress => T::Unstress(xml::Unstress {
            attributes: Default::default(),
            content: (),
        }),
        Articulation::SoftAccent => T::SoftAccent(xml::SoftAccent {
            attributes: Default::default(),
            content: (),
        }),
        // The model doesn't track the mark's shape - use the default glyph.
        Articulation::BreathMark => T::BreathMark(xml::BreathMark {
            attributes: Default::default(),
            content: dt::BreathMarkValue::Comma,
        }),
        Articulation::Caesura => T::Caesura(xml::Caesura {
            attributes: Default::default(),
            content: dt::CaesuraValue::Normal,
        }),
        Articulation::Other(s) => T::OtherArticulation(xml::OtherArticulation {
            attributes: Default::default(),
            content: s.clone(),
        }),
    })
}

/// The model only marks which note a trill line starts on, not how far it
/// extends, so a `WavyLine` is written as a line starting and stopping on
/// that same note.
fn ornaments_to_xml(ornaments: &[Ornament]) -> Vec<xml::OrnamentType> {
    let mut out = Vec::new();
    for ornament in ornaments {
        if *ornament == Ornament::WavyLine {
            for r#type in [dt::StartStopContinue::Start, dt::StartStopContinue::Stop] {
                out.push(xml::OrnamentType::WavyLine(wavy_line(r#type)));
            }
        } else if let Some(t) = ornament_to_xml(ornament) {
            out.push(t);
        }
    }
    out
}

fn wavy_line(r#type: dt::StartStopContinue) -> xml::WavyLine {
    xml::WavyLine {
        attributes: xml::WavyLineAttributes {
            r#type,
            acclerate: None,
            beats: None,
            color: None,
            default_x: None,
            default_y: None,
            last_beat: None,
            number: None,
            placement: None,
            relative_x: None,
            relative_y: None,
            second_beat: None,
            smufl: None,
            start_note: None,
            trill_step: None,
            two_note_turn: None,
        },
        content: (),
    }
}

fn ornament_to_xml(ornament: &Ornament) -> Option<xml::OrnamentType> {
    use xml::OrnamentType as T;
    Some(match ornament {
        Ornament::TrillMark => T::TrillMark(xml::TrillMark {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::Turn => T::Turn(xml::Turn {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::DelayedTurn => T::DelayedTurn(xml::DelayedTurn {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::InvertedTurn => T::InvertedTurn(xml::InvertedTurn {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::DelayedInvertedTurn => T::DelayedInvertedTurn(xml::DelayedInvertedTurn {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::VerticalTurn => T::VerticalTurn(xml::VerticalTurn {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::InvertedVerticalTurn => T::InvertedVerticalTurn(xml::InvertedVerticalTurn {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::Shake => T::Shake(xml::Shake {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::Mordent => T::Mordent(xml::Mordent {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::InvertedMordent => T::InvertedMordent(xml::InvertedMordent {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::Schleifer => T::Schleifer(xml::Schleifer {
            attributes: Default::default(),
            content: (),
        }),
        Ornament::Haydn => T::Haydn(xml::Haydn {
            attributes: Default::default(),
            content: (),
        }),
        // A single-note tremolo (the model has no two-note start/stop form).
        Ornament::Tremolo(marks) => T::Tremolo(xml::Tremolo {
            attributes: xml::TremoloAttributes {
                r#type: Some(dt::TremoloType::Single),
                ..Default::default()
            },
            content: dt::TremoloMarks((*marks).min(8)),
        }),
        // Handled by `ornaments_to_xml`, which needs to emit two elements.
        Ornament::WavyLine => return None,
        Ornament::Other(s) => T::OtherOrnament(xml::OtherOrnament {
            attributes: Default::default(),
            content: s.clone(),
        }),
    })
}

fn dynamic_to_xml(dynamic: &Dynamic) -> Option<xml::DynamicsType> {
    use xml::DynamicsType as T;
    Some(match dynamic {
        Dynamic::P => T::P(xml::P {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Pp => T::Pp(xml::Pp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Ppp => T::Ppp(xml::Ppp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Pppp => T::Pppp(xml::Pppp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Ppppp => T::Ppppp(xml::Ppppp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Pppppp => T::Pppppp(xml::Pppppp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::F => T::F(xml::F {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Ff => T::Ff(xml::Ff {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Fff => T::Fff(xml::Fff {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Ffff => T::Ffff(xml::Ffff {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Fffff => T::Fffff(xml::Fffff {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Ffffff => T::Ffffff(xml::Ffffff {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Mp => T::Mp(xml::Mp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Mf => T::Mf(xml::Mf {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Sf => T::Sf(xml::Sf {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Sfp => T::Sfp(xml::Sfp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Sfpp => T::Sfpp(xml::Sfpp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Fp => T::Fp(xml::Fp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Rf => T::Rf(xml::Rf {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Rfz => T::Rfz(xml::Rfz {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Sfz => T::Sfz(xml::Sfz {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Sffz => T::Sffz(xml::Sffz {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Fz => T::Fz(xml::Fz {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::N => T::N(xml::N {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Pf => T::Pf(xml::Pf {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Sfzp => T::Sfzp(xml::Sfzp {
            attributes: Default::default(),
            content: (),
        }),
        Dynamic::Other(s) => T::OtherDynamics(xml::OtherDynamics {
            attributes: Default::default(),
            content: s.clone(),
        }),
    })
}

fn wedge_to_xml(wedge: Wedge) -> xml::Wedge {
    let r#type = match wedge {
        Wedge::CrescendoStart => dt::WedgeType::Crescendo,
        Wedge::DiminuendoStart => dt::WedgeType::Diminuendo,
        Wedge::Stop => dt::WedgeType::Stop,
    };
    xml::Wedge {
        attributes: xml::WedgeAttributes {
            r#type,
            color: None,
            dash_length: None,
            default_x: None,
            default_y: None,
            id: None,
            line_type: None,
            niente: None,
            number: None,
            relative_x: None,
            relative_y: None,
            space_length: None,
            spread: None,
        },
        content: (),
    }
}

fn empty_note_contents(info: xml::NoteType) -> xml::NoteContents {
    xml::NoteContents {
        info,
        instrument: Vec::new(),
        footnote: None,
        level: None,
        voice: None,
        r#type: None,
        dot: Vec::new(),
        accidental: None,
        time_modification: None,
        stem: None,
        notehead: None,
        notehead_text: None,
        staff: None,
        beam: Vec::new(),
        notations: Vec::new(),
        lyric: Vec::new(),
        play: None,
        listen: None,
    }
}

fn xml_pitch(pitch: &scores_core::Pitch) -> xml::Pitch {
    xml::Pitch {
        attributes: (),
        content: xml::PitchContents {
            step: xml::Step {
                attributes: (),
                content: to_xml_step(pitch.step),
            },
            alter: (pitch.alter != 0).then_some(xml::Alter {
                attributes: (),
                content: dt::Semitones(pitch.alter as i16),
            }),
            octave: xml::Octave {
                attributes: (),
                content: dt::Octave(pitch.octave.clamp(0, 9) as u8),
            },
        },
    }
}

fn xml_ties(tie: Tie) -> Vec<xml::Tie> {
    let make = |t: dt::StartStop| xml::Tie {
        attributes: xml::TieAttributes {
            r#type: t,
            time_only: None,
        },
        content: (),
    };
    match tie {
        Tie::None => vec![],
        Tie::Start => vec![make(dt::StartStop::Start)],
        Tie::Stop => vec![make(dt::StartStop::Stop)],
        Tie::StartStop => vec![make(dt::StartStop::Stop), make(dt::StartStop::Start)],
    }
}

/// The note/rest's own written length (base value + dots, ignoring any
/// tuplet ratio - that is instead conveyed by `<time-modification>`), in
/// integer `<duration>` divisions.
fn sounding_divisions(duration: &Duration) -> u32 {
    (duration.quarter_length() * DIVISIONS as f64)
        .round()
        .max(1.0) as u32
}

fn to_note_type_value(base: NoteValue) -> dt::NoteTypeValue {
    match base {
        NoteValue::Whole => dt::NoteTypeValue::Whole,
        NoteValue::Half => dt::NoteTypeValue::Half,
        NoteValue::Quarter => dt::NoteTypeValue::Quarter,
        NoteValue::Eighth => dt::NoteTypeValue::Eighth,
        NoteValue::Sixteenth => dt::NoteTypeValue::Sixteenth,
        NoteValue::ThirtySecond => dt::NoteTypeValue::ThirtySecond,
        NoteValue::SixtyFourth => dt::NoteTypeValue::SixtyFourth,
    }
}

fn to_xml_step(step: Step) -> dt::Step {
    match step {
        Step::A => dt::Step::A,
        Step::B => dt::Step::B,
        Step::C => dt::Step::C,
        Step::D => dt::Step::D,
        Step::E => dt::Step::E,
        Step::F => dt::Step::F,
        Step::G => dt::Step::G,
    }
}

fn to_xml_mode(mode: Mode) -> dt::Mode {
    match mode {
        Mode::Major => dt::Mode::Major,
        Mode::Minor => dt::Mode::Minor,
    }
}
