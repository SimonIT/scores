//! Converts a `musicxml::elements::ScorePartwise` tree into a [`Score`].
//! See the module docs in [`crate`] for what is (and isn't) mapped.

use musicxml::datatypes as dt;
use musicxml::elements as xml;
use scores_core::{
    Articulation, Barline, BarlineType, ChordKind, ChordRoot, ChordSymbol, Clef, Duration, Ending,
    Event, KeySignature, Measure, MeasureAttributes, Mode, Navigation, Note, NoteValue, Ornament,
    Part, Pitch, Rest, Result, Score, ScoresError, Step, Tempo, Tie, TimeSignature, Tuplet, Voice,
    Wedge,
};
use std::collections::{BTreeMap, HashMap};

pub fn read(data: &[u8]) -> Result<Score> {
    let xml_score = musicxml::read_score_data_partwise(data.to_vec())
        .map_err(|e| ScoresError::Parse(format!("invalid MusicXML: {e}")))?;

    let title = xml_score
        .content
        .movement_title
        .as_ref()
        .map(|t| t.content.clone())
        .or_else(|| {
            xml_score
                .content
                .work
                .as_ref()
                .and_then(|w| w.content.work_title.as_ref().map(|t| t.content.clone()))
        });

    let composer = xml_score.content.identification.as_ref().and_then(|id| {
        id.content
            .creator
            .iter()
            .find(|c| c.attributes.r#type.as_ref().map(|t| t.0.as_str()) == Some("composer"))
            .or_else(|| id.content.creator.first())
            .map(|c| c.content.clone())
    });

    let part_names: HashMap<String, String> = xml_score
        .content
        .part_list
        .content
        .content
        .iter()
        .filter_map(|el| match el {
            xml::PartListElement::ScorePart(sp) => Some((
                sp.attributes.id.0.clone(),
                sp.content.part_name.content.clone(),
            )),
            _ => None,
        })
        .collect();

    let parts = xml_score
        .content
        .part
        .iter()
        .map(|part| {
            let name = part_names.get(&part.attributes.id.0).cloned();
            read_part(part, name)
        })
        .collect();

    Ok(Score {
        title,
        composer,
        parts,
    })
}

#[derive(Default)]
struct PartReadState {
    divisions: f64,
    key_signature: Option<KeySignature>,
    time_signature: Option<TimeSignature>,
    clef: Option<Clef>,
    tempo: Option<Tempo>,
}

/// Per-measure state for reconstructing voices from a flat `<note>`/
/// `<backup>`/`<forward>` stream: `cursor` is the current position (in
/// divisions), `last_note_start` is where the most recent non-chord note
/// began (chord notes share it instead of advancing), and each voice's own
/// reconstructed stream gets a rest inserted whenever it falls behind
/// `cursor` (e.g. after a `<backup>` that only partly rewound this voice).
/// Directions and `<harmony>` elements precede the note they apply to, so
/// they are held as `pending_*` until the next suitable event is placed.
#[derive(Default)]
struct MeasureReadState {
    cursor: i64,
    last_note_start: i64,
    voices: BTreeMap<u8, Vec<Event>>,
    voice_position: BTreeMap<u8, i64>,
    last_voice: Option<u8>,
    pending_dynamic: Option<scores_core::Dynamic>,
    pending_wedge: Option<Wedge>,
    pending_chord_symbol: Option<ChordSymbol>,
    navigation: Vec<Navigation>,
    barline: Barline,
    /// An `<ending type="start">` on this measure: its numbers.
    ending_start: Option<Vec<u8>>,
    /// An `<ending type="stop"/"discontinue">` on this measure: whether it
    /// is closed (`stop`).
    ending_stop: Option<bool>,
}

impl MeasureReadState {
    fn place(&mut self, parsed: ParsedNote, divisions: f64) {
        let stream = self.voices.entry(parsed.voice).or_default();
        let position = self.voice_position.entry(parsed.voice).or_insert(0);
        let start = if parsed.is_chord {
            self.last_note_start
        } else {
            self.cursor
        };

        if !parsed.is_chord && start > *position {
            let gap_quarters = (start - *position) as f64 / divisions;
            stream.push(Event::Rest(Rest::new(
                Duration::from_quarter_length_quantized(gap_quarters),
            )));
            *position = start;
        }

        let mut event = parsed.event;
        if let Event::Note(n) = &mut event {
            if n.dynamic.is_none() {
                n.dynamic = self.pending_dynamic.take();
            }
            if n.wedge.is_none() {
                n.wedge = self.pending_wedge.take();
            }
        }
        // A chord symbol sits over a real (non-grace) note or rest; chord
        // notes after the first can't carry one in the model.
        if !parsed.is_chord && !parsed.grace && self.pending_chord_symbol.is_some() {
            event.set_chord_symbol(self.pending_chord_symbol.take());
        }
        self.last_voice = Some(parsed.voice);

        if parsed.is_chord {
            match (stream.last_mut(), event) {
                (Some(Event::Chord(notes)), Event::Note(n)) => notes.push(n),
                (Some(Event::Note(_)), Event::Note(n)) => {
                    if let Some(Event::Note(previous)) = stream.pop() {
                        stream.push(Event::Chord(vec![previous, n]));
                    }
                }
                (_, other) => stream.push(other),
            }
        } else {
            stream.push(event);
        }

        if !parsed.is_chord {
            self.last_note_start = start;
            if !parsed.grace {
                *position += parsed.duration_divisions as i64;
                self.cursor += parsed.duration_divisions as i64;
            }
        }
    }

    fn into_measure(mut self, number: u32, attributes: MeasureAttributes) -> Measure {
        // A hairpin's stop direction is often written after the measure's
        // last note rather than before the next one - end it on that note.
        if self.pending_wedge == Some(Wedge::Stop)
            && let Some(voice) = self.last_voice
            && let Some(last) = self.voices.get_mut(&voice).and_then(|s| s.last_mut())
        {
            let note = match last {
                Event::Note(n) => Some(n),
                Event::Chord(notes) => notes.first_mut(),
                Event::Rest(_) => None,
            };
            if let Some(note) = note
                && note.wedge.is_none()
            {
                note.wedge = Some(Wedge::Stop);
            }
        }
        let mut voices: Vec<Voice> = self
            .voices
            .into_iter()
            .map(|(number, events)| Voice { number, events })
            .collect();
        if voices.is_empty() {
            voices.push(Voice {
                number: 1,
                events: vec![Event::Rest(Rest::new(Duration::new(NoteValue::Whole)))],
            });
        }
        Measure {
            number,
            attributes,
            voices,
            barline: self.barline,
            navigation: self.navigation,
        }
    }
}

struct ParsedNote {
    voice: u8,
    is_chord: bool,
    grace: bool,
    duration_divisions: f64,
    event: Event,
}

fn read_part(xml_part: &xml::Part, name: Option<String>) -> Part {
    let mut state = PartReadState {
        divisions: 1.0,
        ..Default::default()
    };
    let mut measures = Vec::new();
    // The ending bracket currently open: its first measure's index and its
    // numbers.
    let mut open_ending: Option<(usize, Vec<u8>)> = None;

    for xml_element in &xml_part.content {
        let xml::PartElement::Measure(xml_measure) = xml_element else {
            continue;
        };

        let key_before = state.key_signature;
        let time_before = state.time_signature;
        let clef_before = state.clef;
        let tempo_before = state.tempo;
        let mut measure_state = MeasureReadState::default();

        for el in &xml_measure.content {
            match el {
                xml::MeasureElement::Attributes(attrs) => {
                    apply_attributes(&attrs.content, &mut state)
                }
                xml::MeasureElement::Sound(sound) => measure_state
                    .navigation
                    .extend(apply_sound(sound, &mut state)),
                xml::MeasureElement::Direction(direction) => {
                    // The visible marks and the `<sound>` of one direction
                    // usually say the same thing twice: the sound wins.
                    let mut visible = Vec::new();
                    for direction_type in &direction.content.direction_type {
                        match &direction_type.content {
                            xml::DirectionTypeContents::Dynamics(list) => {
                                if let Some(d) = list
                                    .first()
                                    .and_then(|el| el.content.first())
                                    .and_then(dynamic_from_xml)
                                {
                                    measure_state.pending_dynamic = Some(d);
                                }
                            }
                            xml::DirectionTypeContents::Wedge(wedge) => {
                                if let Some(w) = wedge_from_xml(wedge) {
                                    measure_state.pending_wedge = Some(w);
                                }
                            }
                            xml::DirectionTypeContents::Segno(_) => visible.push(Navigation::Segno),
                            xml::DirectionTypeContents::Coda(_) => visible.push(Navigation::Coda),
                            xml::DirectionTypeContents::Words(words) => visible.extend(
                                words
                                    .iter()
                                    .filter_map(|w| navigation_from_words(&w.content)),
                            ),
                            _ => {}
                        }
                    }
                    let sounded = direction
                        .content
                        .sound
                        .as_ref()
                        .and_then(|sound| apply_sound(sound, &mut state));
                    match sounded {
                        Some(n) => measure_state.navigation.push(n),
                        None => measure_state.navigation.extend(visible),
                    }
                }
                xml::MeasureElement::Harmony(harmony) => {
                    if let Some(symbol) = chord_symbol_from_xml(harmony) {
                        measure_state.pending_chord_symbol = Some(symbol);
                    }
                }
                xml::MeasureElement::Barline(barline) => apply_barline(barline, &mut measure_state),
                xml::MeasureElement::Backup(backup) => {
                    measure_state.cursor -= backup.content.duration.content.0 as i64;
                }
                xml::MeasureElement::Forward(forward) => {
                    measure_state.cursor += forward.content.duration.content.0 as i64;
                }
                xml::MeasureElement::Note(note) => {
                    if let Some(parsed) = read_note(note, state.divisions) {
                        measure_state.place(parsed, state.divisions);
                    }
                }
                _ => {}
            }
        }

        let changed = measures.is_empty()
            || state.key_signature != key_before
            || state.time_signature != time_before
            || state.clef != clef_before
            || state.tempo != tempo_before;
        let attributes = if changed {
            MeasureAttributes {
                time_signature: state.time_signature,
                key_signature: state.key_signature,
                clef: state.clef,
                tempo: state.tempo,
            }
        } else {
            MeasureAttributes::default()
        };

        let number = xml_measure
            .attributes
            .number
            .0
            .trim()
            .parse()
            .unwrap_or((measures.len() + 1) as u32);
        let index = measures.len();
        let ending_start = measure_state.ending_start.take();
        let ending_stop = measure_state.ending_stop.take();
        measures.push(measure_state.into_measure(number, attributes));

        if let Some(numbers) = ending_start {
            // A new bracket before the previous one was stopped ends that
            // one (open) on the measure before.
            if let Some((start, numbers)) = open_ending.take()
                && start < index
            {
                close_ending(&mut measures, start, index - 1, numbers, false);
            }
            open_ending = Some((index, numbers));
        }
        if let Some(closed) = ending_stop
            && let Some((start, numbers)) = open_ending.take()
        {
            close_ending(&mut measures, start, index, numbers, closed);
        }
    }
    // A bracket never stopped runs (open) to the end of the part.
    if let Some((start, numbers)) = open_ending {
        let last = measures.len() - 1;
        close_ending(&mut measures, start, last, numbers, false);
    }

    Part {
        id: xml_part.attributes.id.0.clone(),
        name,
        measures,
    }
}

fn apply_attributes(content: &xml::AttributesContents, state: &mut PartReadState) {
    if let Some(divisions) = &content.divisions {
        state.divisions = (divisions.content.0 as f64).max(1.0);
    }
    if let Some(key) = content.key.first()
        && let xml::KeyContents::Explicit(explicit) = &key.content
    {
        let mode = explicit
            .mode
            .as_ref()
            .map(|m| {
                if m.content == dt::Mode::Minor {
                    Mode::Minor
                } else {
                    Mode::Major
                }
            })
            .unwrap_or(Mode::Major);
        state.key_signature = Some(KeySignature {
            fifths: explicit.fifths.content.0,
            mode,
        });
    }
    if let Some(time) = content.time.first()
        && let Some(beat) = time.content.beats.first()
        && let (Ok(beats), Ok(beat_type)) =
            (beat.beats.content.parse(), beat.beat_type.content.parse())
    {
        state.time_signature = Some(TimeSignature { beats, beat_type });
    }
    if let Some(clef) = content.clef.first() {
        state.clef = Some(clef_from_xml(clef));
    }
}

/// Sets `<sound>`'s tempo on `state` and returns the navigation mark it
/// encodes, if any.
fn apply_sound(sound: &xml::Sound, state: &mut PartReadState) -> Option<Navigation> {
    if let Some(tempo) = &sound.attributes.tempo {
        let bpm = tempo.0 as f32;
        if bpm > 0.0 {
            state.tempo = Some(Tempo { bpm });
        }
    }

    let dacapo = sound.attributes.dacapo == Some(dt::YesNo::Yes);
    let dalsegno = sound.attributes.dalsegno.is_some();
    let fine = sound.attributes.fine.is_some();
    let tocoda = sound.attributes.tocoda.is_some();

    let resolved = match (dacapo, dalsegno, fine, tocoda) {
        (true, _, true, _) => Some(Navigation::DaCapoAlFine),
        (true, _, _, true) => Some(Navigation::DaCapoAlCoda),
        (true, _, false, false) => Some(Navigation::DaCapo),
        (_, true, true, _) => Some(Navigation::DalSegnoAlFine),
        (_, true, _, true) => Some(Navigation::DalSegnoAlCoda),
        (false, true, false, false) => Some(Navigation::DalSegno),
        (false, false, true, false) => Some(Navigation::Fine),
        (false, false, false, true) => Some(Navigation::ToCoda),
        _ => None,
    };
    if resolved.is_some() {
        resolved
    } else if sound.attributes.segno.is_some() {
        Some(Navigation::Segno)
    } else if sound.attributes.coda.is_some() {
        Some(Navigation::Coda)
    } else {
        None
    }
}

/// Records the bracket that started at `measures[start]` and stopped at
/// `measures[last]`.
fn close_ending(
    measures: &mut [Measure],
    start: usize,
    last: usize,
    numbers: Vec<u8>,
    closed: bool,
) {
    measures[start].barline.ending = Some(Ending {
        numbers,
        measures: (last - start + 1) as u32,
        closed,
    });
}

/// Maps a direction's text (`D.S. al Coda`, `Fine`, ...) to a navigation
/// mark, for files that write these as plain `<words>` without a `<sound>`.
fn navigation_from_words(text: &str) -> Option<Navigation> {
    let normalized = text
        .to_lowercase()
        .replace('.', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(match normalized.as_str() {
        "dc" | "da capo" => Navigation::DaCapo,
        "dc al fine" | "da capo al fine" => Navigation::DaCapoAlFine,
        "dc al coda" | "da capo al coda" => Navigation::DaCapoAlCoda,
        "ds" | "dal segno" => Navigation::DalSegno,
        "ds al fine" | "dal segno al fine" => Navigation::DalSegnoAlFine,
        "ds al coda" | "dal segno al coda" => Navigation::DalSegnoAlCoda,
        "fine" => Navigation::Fine,
        "to coda" => Navigation::ToCoda,
        "coda" => Navigation::Coda,
        "segno" => Navigation::Segno,
        _ => return None,
    })
}

/// The model's barline is the one at the *end* of the measure, so only a
/// right-hand `<barline>` sets its style (a left one typically just carries
/// a forward repeat's heavy-light). An ending bracket's start/stop is only
/// noted here; [`read_part`] pairs them up into a span.
fn apply_barline(barline: &xml::Barline, measure: &mut MeasureReadState) {
    let out = &mut measure.barline;
    let is_left = barline.attributes.location == Some(dt::RightLeftMiddle::Left);
    if let Some(style) = &barline.content.bar_style
        && !is_left
    {
        out.kind = barline_type_from_xml(&style.content);
    }
    if let Some(repeat) = &barline.content.repeat {
        match repeat.attributes.direction {
            dt::BackwardForward::Forward => out.repeat_start = true,
            dt::BackwardForward::Backward => {
                let times = repeat.attributes.times.as_ref().map(|t| t.0).unwrap_or(2);
                out.repeat_end = Some(times.max(2));
            }
        }
    }
    if let Some(ending) = &barline.content.ending {
        let numbers = ending
            .attributes
            .number
            .0
            .split(',')
            .filter_map(|n| n.trim().parse().ok())
            .collect();
        match ending.attributes.r#type {
            dt::StartStopDiscontinue::Start => measure.ending_start = Some(numbers),
            dt::StartStopDiscontinue::Stop => measure.ending_stop = Some(true),
            dt::StartStopDiscontinue::Discontinue => measure.ending_stop = Some(false),
        }
    }
}

fn barline_type_from_xml(style: &dt::BarStyle) -> BarlineType {
    match style {
        dt::BarStyle::Dashed => BarlineType::Dashed,
        dt::BarStyle::Dotted => BarlineType::Dotted,
        dt::BarStyle::Heavy => BarlineType::Heavy,
        dt::BarStyle::HeavyHeavy => BarlineType::HeavyHeavy,
        dt::BarStyle::HeavyLight => BarlineType::HeavyLight,
        dt::BarStyle::LightHeavy => BarlineType::LightHeavy,
        dt::BarStyle::LightLight => BarlineType::LightLight,
        dt::BarStyle::None => BarlineType::NoBarline,
        dt::BarStyle::Regular => BarlineType::Regular,
        dt::BarStyle::Short => BarlineType::Short,
        dt::BarStyle::Tick => BarlineType::Tick,
    }
}

fn clef_from_xml(clef: &xml::Clef) -> Clef {
    let line = clef.content.line.as_ref().map(|l| l.content.0);
    match clef.content.sign.content {
        dt::ClefSign::G => Clef::Treble,
        dt::ClefSign::F => Clef::Bass,
        dt::ClefSign::C => {
            if line == Some(4) {
                Clef::Tenor
            } else {
                Clef::Alto
            }
        }
        dt::ClefSign::Percussion => Clef::Percussion,
        _ => Clef::Treble,
    }
}

/// Reads one `<note>`, whatever kind it is (normal/cue/grace). Grace notes
/// have no `<duration>` element at all (they borrow time visually, not
/// really), so they get a nominal notated duration and `grace: true`
/// instead. A cue+grace combination (vanishingly rare) is dropped.
fn read_note(note: &xml::Note, divisions: f64) -> Option<ParsedNote> {
    let voice = note
        .content
        .voice
        .as_ref()
        .and_then(|v| v.content.parse().ok())
        .unwrap_or(1);
    let (articulations, ornaments, fermata, dynamic) = extract_notations(&note.content.notations);
    let note_type = note.content.r#type.as_ref().map(|t| &t.content);
    let dots = note.content.dot.len();
    let tuplet = note.content.time_modification.as_ref().map(|tm| {
        (
            tm.content.actual_notes.content.0 as u8,
            tm.content.normal_notes.content.0 as u8,
        )
    });

    let (is_chord, audible, tie, duration_divisions, grace) = match &note.content.info {
        xml::NoteType::Normal(info) => (
            info.chord.is_some(),
            &info.audible,
            tie_from_xml(&info.tie),
            info.duration.content.0 as f64,
            false,
        ),
        xml::NoteType::Cue(info) => (
            info.chord.is_some(),
            &info.audible,
            Tie::None,
            info.duration.content.0 as f64,
            false,
        ),
        xml::NoteType::Grace(grace_info) => {
            let xml::GraceType::Normal(info) = &grace_info.info else {
                return None;
            };
            (
                info.chord.is_some(),
                &info.audible,
                tie_from_xml(&info.tie),
                0.0,
                true,
            )
        }
    };

    // Grace notes still carry a notated `<type>`; without one, an eighth
    // is the conventional default.
    let duration = if grace {
        match note_type {
            Some(note_type) => Duration::dotted(from_note_type_value(note_type), dots as u8),
            None => Duration::new(NoteValue::Eighth),
        }
    } else {
        xml_duration_to_model(duration_divisions / divisions, note_type, dots, tuplet)
    };

    let mut event = match audible {
        xml::AudibleType::Pitch(pitch) => Event::Note(Note::new(model_pitch(pitch), duration, tie)),
        xml::AudibleType::Rest(_) | xml::AudibleType::Unpitched(_) => {
            Event::Rest(Rest::new(duration))
        }
    };
    match &mut event {
        Event::Note(n) => {
            n.grace = grace;
            n.fermata = fermata;
            n.articulations = articulations;
            n.ornaments = ornaments;
            n.dynamic = dynamic;
        }
        Event::Rest(r) => r.fermata = fermata,
        Event::Chord(_) => {}
    }

    Some(ParsedNote {
        voice,
        is_chord,
        grace,
        duration_divisions,
        event,
    })
}

/// A note's articulations, ornaments, fermata and any dynamic written as a
/// notation (rather than the usual `<direction>`).
type Notations = (
    Vec<Articulation>,
    Vec<Ornament>,
    bool,
    Option<scores_core::Dynamic>,
);

fn extract_notations(blocks: &[xml::Notations]) -> Notations {
    let mut articulations = Vec::new();
    let mut ornaments = Vec::new();
    let mut fermata = false;
    let mut dynamic = None;
    for block in blocks {
        for item in &block.content.notations {
            match item {
                xml::NotationContentTypes::Articulations(a) => {
                    articulations.extend(a.content.iter().filter_map(articulation_from_xml));
                }
                xml::NotationContentTypes::Ornaments(o) => {
                    ornaments.extend(o.content.ornaments.iter().filter_map(ornament_from_xml));
                }
                xml::NotationContentTypes::Fermata(_) => fermata = true,
                xml::NotationContentTypes::Dynamics(d) if dynamic.is_none() => {
                    dynamic = d.content.first().and_then(dynamic_from_xml);
                }
                _ => {}
            }
        }
    }
    (articulations, ornaments, fermata, dynamic)
}

fn articulation_from_xml(t: &xml::ArticulationsType) -> Option<Articulation> {
    Some(match t {
        xml::ArticulationsType::Accent(_) => Articulation::Accent,
        xml::ArticulationsType::StrongAccent(_) => Articulation::StrongAccent,
        xml::ArticulationsType::Staccato(_) => Articulation::Staccato,
        xml::ArticulationsType::Tenuto(_) => Articulation::Tenuto,
        xml::ArticulationsType::DetachedLegato(_) => Articulation::DetachedLegato,
        xml::ArticulationsType::Staccatissimo(_) => Articulation::Staccatissimo,
        xml::ArticulationsType::Spiccato(_) => Articulation::Spiccato,
        xml::ArticulationsType::Scoop(_) => Articulation::Scoop,
        xml::ArticulationsType::Plop(_) => Articulation::Plop,
        xml::ArticulationsType::Doit(_) => Articulation::Doit,
        xml::ArticulationsType::Falloff(_) => Articulation::Falloff,
        xml::ArticulationsType::BreathMark(_) => Articulation::BreathMark,
        xml::ArticulationsType::Caesura(_) => Articulation::Caesura,
        xml::ArticulationsType::Stress(_) => Articulation::Stress,
        xml::ArticulationsType::Unstress(_) => Articulation::Unstress,
        xml::ArticulationsType::SoftAccent(_) => Articulation::SoftAccent,
        xml::ArticulationsType::OtherArticulation(o) => Articulation::Other(o.content.clone()),
    })
}

fn ornament_from_xml(t: &xml::OrnamentType) -> Option<Ornament> {
    Some(match t {
        xml::OrnamentType::TrillMark(_) => Ornament::TrillMark,
        xml::OrnamentType::Turn(_) => Ornament::Turn,
        xml::OrnamentType::DelayedTurn(_) => Ornament::DelayedTurn,
        xml::OrnamentType::InvertedTurn(_) => Ornament::InvertedTurn,
        xml::OrnamentType::DelayedInvertedTurn(_) => Ornament::DelayedInvertedTurn,
        xml::OrnamentType::VerticalTurn(_) => Ornament::VerticalTurn,
        xml::OrnamentType::InvertedVerticalTurn(_) => Ornament::InvertedVerticalTurn,
        xml::OrnamentType::Shake(_) => Ornament::Shake,
        // Only the start of a trill line marks the note; its stop/continue
        // elements on later notes carry nothing the model tracks.
        xml::OrnamentType::WavyLine(w) => {
            if w.attributes.r#type != dt::StartStopContinue::Start {
                return None;
            }
            Ornament::WavyLine
        }
        xml::OrnamentType::Mordent(_) => Ornament::Mordent,
        xml::OrnamentType::InvertedMordent(_) => Ornament::InvertedMordent,
        xml::OrnamentType::Schleifer(_) => Ornament::Schleifer,
        xml::OrnamentType::Tremolo(t) => Ornament::Tremolo(t.content.0),
        xml::OrnamentType::Haydn(_) => Ornament::Haydn,
        xml::OrnamentType::OtherOrnament(o) => Ornament::Other(o.content.clone()),
    })
}

fn dynamic_from_xml(d: &xml::DynamicsType) -> Option<scores_core::Dynamic> {
    use scores_core::Dynamic as D;
    Some(match d {
        xml::DynamicsType::P(_) => D::P,
        xml::DynamicsType::Pp(_) => D::Pp,
        xml::DynamicsType::Ppp(_) => D::Ppp,
        xml::DynamicsType::Pppp(_) => D::Pppp,
        xml::DynamicsType::Ppppp(_) => D::Ppppp,
        xml::DynamicsType::Pppppp(_) => D::Pppppp,
        xml::DynamicsType::F(_) => D::F,
        xml::DynamicsType::Ff(_) => D::Ff,
        xml::DynamicsType::Fff(_) => D::Fff,
        xml::DynamicsType::Ffff(_) => D::Ffff,
        xml::DynamicsType::Fffff(_) => D::Fffff,
        xml::DynamicsType::Ffffff(_) => D::Ffffff,
        xml::DynamicsType::Mp(_) => D::Mp,
        xml::DynamicsType::Mf(_) => D::Mf,
        xml::DynamicsType::Sf(_) => D::Sf,
        xml::DynamicsType::Sfp(_) => D::Sfp,
        xml::DynamicsType::Sfpp(_) => D::Sfpp,
        xml::DynamicsType::Fp(_) => D::Fp,
        xml::DynamicsType::Rf(_) => D::Rf,
        xml::DynamicsType::Rfz(_) => D::Rfz,
        xml::DynamicsType::Sfz(_) => D::Sfz,
        xml::DynamicsType::Sffz(_) => D::Sffz,
        xml::DynamicsType::Fz(_) => D::Fz,
        xml::DynamicsType::N(_) => D::N,
        xml::DynamicsType::Pf(_) => D::Pf,
        xml::DynamicsType::Sfzp(_) => D::Sfzp,
        xml::DynamicsType::OtherDynamics(o) => D::Other(o.content.clone()),
    })
}

fn wedge_from_xml(wedge: &xml::Wedge) -> Option<Wedge> {
    match wedge.attributes.r#type {
        dt::WedgeType::Crescendo => Some(Wedge::CrescendoStart),
        dt::WedgeType::Diminuendo => Some(Wedge::DiminuendoStart),
        dt::WedgeType::Stop => Some(Wedge::Stop),
        dt::WedgeType::Continue => None,
    }
}

/// Reads a `<harmony>` chord symbol. Only the first harmony-chord of a
/// stacked one is used, and a numeral/function-only (Roman numeral
/// analysis) harmony is skipped. A harmony with no root at all is the
/// "N.C." form (`<kind>none</kind>` alone). `<degree>` alterations are
/// folded into the suffix text (`7` + flat 9 = `7b9`), which becomes a
/// named [`ChordKind`] where [`ChordKind::from_suffix`] recognizes it.
fn chord_symbol_from_xml(harmony: &xml::Harmony) -> Option<ChordSymbol> {
    let Some(chord) = harmony.content.harmony.first() else {
        return Some(ChordSymbol::no_chord());
    };
    let kind_value = &chord.kind.content;
    if *kind_value == dt::KindValue::None {
        return Some(ChordSymbol::no_chord());
    }
    let root = chord.root.as_ref()?;
    let root = ChordRoot::new(
        from_xml_step(&root.content.root_step.content),
        root.content
            .root_alter
            .as_ref()
            .map(|a| a.content.0.clamp(-2, 2) as i8)
            .unwrap_or(0),
    );
    let bass = chord.bass.as_ref().map(|b| {
        ChordRoot::new(
            from_xml_step(&b.content.bass_step.content),
            b.content
                .bass_alter
                .as_ref()
                .map(|a| a.content.0.clamp(-2, 2) as i8)
                .unwrap_or(0),
        )
    });

    let text = chord.kind.attributes.text.as_ref().map(|t| t.0.as_str());
    let mut kind = chord_kind_from_xml(kind_value, text);
    if !chord.degree.is_empty() {
        let mut suffix = kind.suffix().to_string();
        for degree in &chord.degree {
            suffix.push_str(&degree_text(&degree.content));
        }
        kind = ChordKind::from_suffix(&suffix);
    }
    Some(ChordSymbol { root, kind, bass })
}

/// Maps a `kind-value` to a [`ChordKind`]. Values with no model equivalent
/// (`other`, `pedal`, the augmented-sixth/Neapolitan/Tristan names) become
/// [`ChordKind::Other`] holding the kind's display `text` if it has one.
fn chord_kind_from_xml(value: &dt::KindValue, text: Option<&str>) -> ChordKind {
    use dt::KindValue as K;
    match value {
        K::Major => ChordKind::Major,
        K::Minor => ChordKind::Minor,
        K::Augmented => ChordKind::Augmented,
        K::Diminished => ChordKind::Diminished,
        K::Dominant => ChordKind::Dominant,
        K::MajorSeventh => ChordKind::MajorSeventh,
        K::MinorSeventh => ChordKind::MinorSeventh,
        K::DiminishedSeventh => ChordKind::DiminishedSeventh,
        K::AugmentedSeventh => ChordKind::AugmentedSeventh,
        K::HalfDiminished => ChordKind::HalfDiminished,
        K::MajorMinor => ChordKind::MajorMinor,
        K::MajorSixth => ChordKind::MajorSixth,
        K::MinorSixth => ChordKind::MinorSixth,
        K::DominantNinth => ChordKind::DominantNinth,
        K::MajorNinth => ChordKind::MajorNinth,
        K::MinorNinth => ChordKind::MinorNinth,
        K::Dominant11th => ChordKind::Dominant11th,
        K::Major11th => ChordKind::Major11th,
        K::Minor11th => ChordKind::Minor11th,
        K::Dominant13th => ChordKind::Dominant13th,
        K::Major13th => ChordKind::Major13th,
        K::Minor13th => ChordKind::Minor13th,
        K::SuspendedSecond => ChordKind::SuspendedSecond,
        K::SuspendedFourth => ChordKind::SuspendedFourth,
        K::Power => ChordKind::Power,
        K::None => ChordKind::NoChord,
        K::Other => ChordKind::Other(text.unwrap_or_default().to_string()),
        K::Pedal => ChordKind::Other(text.unwrap_or("pedal").to_string()),
        K::Neapolitan => ChordKind::Other(text.unwrap_or("Neapolitan").to_string()),
        K::Italian => ChordKind::Other(text.unwrap_or("Italian").to_string()),
        K::French => ChordKind::Other(text.unwrap_or("French").to_string()),
        K::German => ChordKind::Other(text.unwrap_or("German").to_string()),
        K::Tristan => ChordKind::Other(text.unwrap_or("Tristan").to_string()),
    }
}

/// Lead-sheet text for one `<degree>`: `add` with an alteration is written
/// as just the altered degree (`b9`), without one as `add9`; `subtract` as
/// `no3`.
fn degree_text(degree: &xml::DegreeContents) -> String {
    let value = degree.degree_value.content.0;
    let alter = degree.degree_alter.content.0;
    let accidental = match alter.signum() {
        1 => "#".repeat(alter as usize),
        -1 => "b".repeat(alter.unsigned_abs() as usize),
        _ => String::new(),
    };
    match degree.degree_type.content {
        dt::DegreeTypeValue::Add if alter == 0 => format!("add{value}"),
        dt::DegreeTypeValue::Add | dt::DegreeTypeValue::Alter => format!("{accidental}{value}"),
        dt::DegreeTypeValue::Subtract => format!("no{value}"),
    }
}

/// The `<type>`/`<dot>`/`<time-modification>` elements are the source of
/// truth for notated duration. Some files omit `<type>` entirely (e.g. some
/// whole-measure rests) - fall back to quantizing the raw tick length
/// instead of assuming a particular note value.
fn xml_duration_to_model(
    quarter_length: f64,
    note_type: Option<&dt::NoteTypeValue>,
    dots: usize,
    tuplet: Option<(u8, u8)>,
) -> Duration {
    match note_type {
        Some(note_type) => Duration {
            base: from_note_type_value(note_type),
            dots: dots.min(u8::MAX as usize) as u8,
            tuplet: tuplet.map(|(actual, normal)| Tuplet { actual, normal }),
        },
        None => Duration::from_quarter_length_quantized(quarter_length),
    }
}

fn from_note_type_value(value: &dt::NoteTypeValue) -> NoteValue {
    match value {
        dt::NoteTypeValue::Whole => NoteValue::Whole,
        dt::NoteTypeValue::Half => NoteValue::Half,
        dt::NoteTypeValue::Quarter => NoteValue::Quarter,
        dt::NoteTypeValue::Eighth => NoteValue::Eighth,
        dt::NoteTypeValue::Sixteenth => NoteValue::Sixteenth,
        dt::NoteTypeValue::ThirtySecond => NoteValue::ThirtySecond,
        dt::NoteTypeValue::SixtyFourth => NoteValue::SixtyFourth,
        dt::NoteTypeValue::Maxima | dt::NoteTypeValue::Long | dt::NoteTypeValue::Breve => {
            NoteValue::Whole
        }
        _ => NoteValue::SixtyFourth,
    }
}

fn model_pitch(pitch: &xml::Pitch) -> Pitch {
    Pitch {
        step: from_xml_step(&pitch.content.step.content),
        alter: pitch
            .content
            .alter
            .as_ref()
            .map(|a| a.content.0 as i8)
            .unwrap_or(0),
        octave: pitch.content.octave.content.0 as i8,
    }
}

fn from_xml_step(step: &dt::Step) -> Step {
    match step {
        dt::Step::A => Step::A,
        dt::Step::B => Step::B,
        dt::Step::C => Step::C,
        dt::Step::D => Step::D,
        dt::Step::E => Step::E,
        dt::Step::F => Step::F,
        dt::Step::G => Step::G,
    }
}

fn tie_from_xml(ties: &[xml::Tie]) -> Tie {
    let has_start = ties
        .iter()
        .any(|t| t.attributes.r#type == dt::StartStop::Start);
    let has_stop = ties
        .iter()
        .any(|t| t.attributes.r#type == dt::StartStop::Stop);
    match (has_start, has_stop) {
        (true, true) => Tie::StartStop,
        (true, false) => Tie::Start,
        (false, true) => Tie::Stop,
        (false, false) => Tie::None,
    }
}
