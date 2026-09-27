//! Builds a `guitarpro::model::optimized::LoadedScore` from a [`Score`] and
//! serializes it to an `.mscz` archive via `guitarpro::write_mscz`. See the
//! module docs in [`crate`] for what actually survives the round trip.

use crate::convert::{
    ArticulationMark, MscxBarline, NavigationMark, articulation_to_mscx, barline_to_mscx,
    clef_to_optimized, duration_to_optimized, dynamic_name, dynamic_velocity, hairpin_subtype,
    is_mscx_dynamic_subtype, key_signature_to_optimized, navigation_to_mscx, ornament_symbol,
    pitch_to_optimized, tpc_from_root, tremolo_subtype,
};
use crate::mscx::{self, BeatPlan, MeasurePlan, Plan, VoicePlan, xml_escape};
use guitarpro::model::optimized::LoadedScore;
use guitarpro::model::optimized::beat::{Beat as OptBeat, Voice as OptVoice};
use guitarpro::model::optimized::global::{
    Instrument, InstrumentId, InstrumentKind, Score as OptScore, StaffId, TrackId,
};
use guitarpro::model::optimized::metadata::{Metadata, TimeSignature as OptTimeSignature};
use guitarpro::model::optimized::note::{Note as OptNote, TieType};
use guitarpro::model::optimized::timeline::{JumpKind, MeasureDef, NavigationEvent};
use guitarpro::model::optimized::track::{MeasureData, StaffDef, StaffDisplay, Track};
use guitarpro::{MsczArchive, MsczEntry, MsczFile};
use scores_core::{
    Articulation, ChordKind, ChordSymbol, Duration, Dynamic, Event, Measure, Note, Ornament, Part,
    Result, Score, ScoresError, Tie, TimeSignature, Voice, Wedge,
};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

const TICK_RESOLUTION: u16 = 960;
const MSCX_PATH: &str = "score.mscx";

pub fn write(score: &Score) -> Result<Vec<u8>> {
    if score.parts.is_empty() {
        return Err(ScoresError::Unsupported(
            "cannot write an empty score to MuseScore format".into(),
        ));
    }

    let loaded = build_loaded_score(score);
    let mut mscx = guitarpro::convert::mscz::loaded_score_to_mscx(&loaded);
    // Splice in everything the converter doesn't write (see `crate::mscx`).
    let plan = build_plan(score, loaded.score.timeline.len());
    mscx.raw_xml = mscx::inject(&mscx.raw_xml, &plan)?;

    let container_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container>\n  <rootfiles>\n    <rootfile full-path=\"{MSCX_PATH}\"/>\n  </rootfiles>\n</container>\n"
    );
    let archive = MsczArchive {
        rootfiles: vec![MSCX_PATH.to_string()],
        entries: vec![
            MsczEntry {
                path: "META-INF/container.xml".to_string(),
                data: container_xml.into_bytes(),
            },
            MsczEntry {
                path: MSCX_PATH.to_string(),
                data: mscx.raw_xml.clone().into_bytes(),
            },
        ],
    };
    let file = MsczFile { archive, mscx };

    guitarpro::write_mscz(&file)
        .map_err(|e| ScoresError::Write(format!("failed to serialize MuseScore file: {e}")))
}

fn build_loaded_score(score: &Score) -> LoadedScore {
    let measure_count = score
        .parts
        .iter()
        .map(|p| p.measures.len())
        .max()
        .unwrap_or(0);

    let instruments: Vec<Instrument> = score
        .parts
        .iter()
        .map(|part| Instrument {
            name: part.name.clone().unwrap_or_else(|| part.id.clone()),
            abbreviation: None,
            instrument_sound: None,
            midi_channel: 0,
            midi_program: 0,
            midi_bank: None,
            volume: None,
            pan: None,
            kind: InstrumentKind::Pitched,
            transpose: None,
            gp_strings: Vec::new(),
        })
        .collect();

    let staves: Vec<StaffDef> = score
        .parts
        .iter()
        .map(|part| StaffDef {
            clef: clef_to_optimized(
                part.measures
                    .first()
                    .and_then(|m| m.attributes.clef)
                    .unwrap_or_default(),
            ),
            display: StaffDisplay::Notation,
        })
        .collect();

    let tracks: Vec<Track> = score
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| build_track(part, index, measure_count))
        .collect();

    let timeline: Vec<MeasureDef> = (0..measure_count)
        .map(|index| build_measure_def(score, index))
        .collect();

    let first_attrs = score
        .parts
        .first()
        .and_then(|p| p.measures.first())
        .map(|m| &m.attributes);
    let time_signature = first_attrs
        .and_then(|a| a.time_signature)
        .unwrap_or_default();
    let key_signature = first_attrs
        .and_then(|a| a.key_signature)
        .unwrap_or_default();
    let master_tempo = first_attrs.and_then(|a| a.tempo).unwrap_or_default().bpm;

    let metadata = Metadata {
        work: None,
        movement_number: None,
        title: score.title.clone().unwrap_or_default(),
        artist: None,
        album: None,
        composer: score.composer.clone(),
        year: None,
        copyright: None,
        identification: None,
        credits: Vec::new(),
        master_tempo,
        time_signature: OptTimeSignature {
            numerator: time_signature.beats,
            denominator: time_signature.beat_type,
        },
        key_signature: key_signature_to_optimized(key_signature),
        chords: Vec::new(),
        scale_hint: None,
    };

    LoadedScore {
        score: OptScore {
            metadata,
            instruments,
            staves,
            tracks,
            groups: Vec::new(),
            timeline,
            lyric_lines: Vec::new(),
            lyric_projections: Vec::new(),
            defaults: None,
        },
        layout: None,
    }
}

fn build_track(part: &Part, index: usize, measure_count: usize) -> Track {
    let track_id = TrackId(index as u8);
    let mut measures: BTreeMap<_, _> = BTreeMap::new();

    for measure_index in 0..measure_count {
        let Some(measure) = part.measures.get(measure_index) else {
            continue;
        };
        let idx = guitarpro::model::optimized::global::MeasureIndex(measure_index as u16);

        let mut voices: HashMap<u8, OptVoice> = HashMap::new();
        for voice in &measure.voices {
            let voice_id = voice.number.saturating_sub(1);
            let beats = voice.events.iter().map(event_to_beat).collect();
            voices.insert(voice_id, OptVoice { voice_id, beats });
        }

        measures.insert(
            idx,
            MeasureData {
                measure_index: idx,
                track_id,
                repeat: None,
                voices,
                gp_line_break: 0,
                gp_simile_mark: None,
            },
        );
    }

    Track {
        id: track_id,
        name: part.name.clone().unwrap_or_else(|| part.id.clone()),
        instrument: InstrumentId(index as u8),
        staves: vec![StaffId(index as u8)],
        measures,
    }
}

/// Repeats are shared across the whole system (like tempo/time/key
/// signature), so - matching every other multi-part plugin in this
/// workspace - the first part with a measure at this index wins.
fn shared_measure(score: &Score, index: usize) -> Option<&Measure> {
    score.parts.iter().find_map(|p| p.measures.get(index))
}

fn build_measure_def(score: &Score, index: usize) -> MeasureDef {
    let shared = shared_measure(score, index);
    let attrs = shared.map(|m| &m.attributes);
    let barline = shared.map(|m| &m.barline);
    let idx = guitarpro::model::optimized::global::MeasureIndex(index as u16);

    let mut navigation = Vec::new();
    if barline.is_some_and(|b| b.repeat_start) {
        navigation.push(NavigationEvent {
            measure_index: idx,
            kind: JumpKind::RepeatOpen,
            repeat_count: None,
            volta: None,
            volta_last: false,
        });
    }
    if let Some(times) = barline.and_then(|b| b.repeat_end) {
        navigation.push(NavigationEvent {
            measure_index: idx,
            kind: JumpKind::RepeatClose,
            repeat_count: Some(times.min(u8::MAX as u32) as u8),
            volta: None,
            volta_last: false,
        });
    }

    let time_signature = attrs.and_then(|a| a.time_signature);
    let beats = time_signature.unwrap_or_default().beats;
    let beat_type = time_signature.unwrap_or_default().beat_type;
    let duration_ticks = (beats as u32) * (TICK_RESOLUTION as u32 * 4 / beat_type.max(1) as u32);

    MeasureDef {
        index: idx,
        tempo: attrs.and_then(|a| a.tempo).map(|t| t.bpm),
        time_signature: time_signature.map(|ts| OptTimeSignature {
            numerator: ts.beats,
            denominator: ts.beat_type,
        }),
        key_signature: attrs
            .and_then(|a| a.key_signature)
            .map(key_signature_to_optimized),
        marker: None,
        navigation,
        tick_resolution: TICK_RESOLUTION,
        duration_ticks,
        barline_left: None,
        barline_right: None,
        gp_beams: None,
        gp_fermatas: Vec::new(),
        gp_free_time: false,
    }
}

fn event_to_beat(event: &Event) -> OptBeat {
    let (notes, duration) = match event {
        Event::Rest(rest) => (Vec::new(), rest.duration),
        Event::Note(note) => (vec![note_to_optimized(note)], note.duration),
        Event::Chord(notes) => (
            notes.iter().map(note_to_optimized).collect(),
            notes.first().map(|n| n.duration).unwrap_or_default(),
        ),
    };

    OptBeat {
        tick_offset: 0,
        duration: duration_to_optimized(&duration),
        notes,
        events: Vec::new(),
        dynamic: None,
        slur: None,
        lyric: None,
        beam_group: None,
        tuplet: None,
        beams: Vec::new(),
        grace_notes: Vec::new(),
        cue: false,
        chord: None,
        gp_empty: false,
        gp_rest: matches!(event, Event::Rest(_)),
        gp_vibrato: false,
        gp_fade_in: false,
        gp_stroke: None,
        gp_pick_stroke: None,
        gp_beat_flags2: None,
        gp_break_secondary: None,
        gp_slap_effect: None,
        gp_rasgueado: false,
        gp_text: String::new(),
        gp_mix_table: None,
        gp_tremolo_bar: None,
        gp_chord: None,
    }
}

/// A [`Tie::StartStop`] note (tied to both neighbours) can't be represented
/// - the optimized model only tracks one tie direction per note - so it's
///   written as `Start`, keeping the tie going forward and dropping the
///   incoming tie from the previous note.
fn note_to_optimized(note: &Note) -> OptNote {
    OptNote {
        pitch: Some(pitch_to_optimized(note.pitch)),
        string: None,
        fret: None,
        tie: match note.tie {
            Tie::Start | Tie::StartStop => Some(TieType::Start),
            Tie::Stop => Some(TieType::End),
            Tie::None => None,
        },
        techniques: Vec::new(),
        ornaments: Vec::new(),
        articulations: Vec::new(),
        left_finger: None,
        right_finger: None,
        notehead: None,
        stem: None,
        accidental: None,
        arpeggiate: None,
        display_pitch: None,
        gp_harmonic: None,
        gp_grace: None,
        gp_bend: None,
        gp_trill: None,
        gp_ghost: false,
        gp_duration_percent: 1.0,
        gp_swap_accidentals: false,
        gp_velocity: None,
        gp_note_type_raw: None,
        gp_is_rest: false,
        gp_ornament: None,
        gp_note_duration: None,
        gp_note_tuplet: None,
    }
}

// ---------------------------------------------------------------------------
// Raw-MSCX extras (see `crate::mscx`)
// ---------------------------------------------------------------------------

/// Written note length in 1/`WHOLE_TICKS` of a whole note - fine enough for
/// a 64th with several dots. Only written values matter here: `guitarpro`
/// writes no tuplets, and MuseScore computes positions from what's written.
const WHOLE_TICKS: i64 = 1 << 12;

fn written_ticks(duration: &Duration) -> i64 {
    let mut addend = (WHOLE_TICKS as f64 * duration.base.quarter_length() / 4.0) as i64;
    let mut total = addend;
    for _ in 0..duration.dots {
        addend /= 2;
        total += addend;
    }
    total
}

fn event_ticks(event: &Event) -> i64 {
    if is_grace(event) {
        0
    } else {
        written_ticks(&event.duration())
    }
}

fn is_grace(event: &Event) -> bool {
    match event {
        Event::Note(n) => n.grace,
        Event::Chord(notes) => notes.first().is_some_and(|n| n.grace),
        Event::Rest(_) => false,
    }
}

fn fraction_text(ticks: i64) -> String {
    fn gcd(a: i64, b: i64) -> i64 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let divisor = gcd(ticks.abs(), WHOLE_TICKS).max(1);
    format!("{}/{}", ticks / divisor, WHOLE_TICKS / divisor)
}

/// A spanner's `<next>`/`<prev>` location, relative to its own position.
/// MuseScore pairs a spanner's start and end elements by these, so they
/// must match where the other element actually is.
fn location_xml(tag: &str, measures: i64, ticks: i64, voices: i64) -> String {
    let mut out = format!("<{tag}><location>");
    if voices != 0 {
        write!(out, "<voices>{voices}</voices>").ok();
    }
    if measures != 0 {
        write!(out, "<measures>{measures}</measures>").ok();
    }
    if ticks != 0 {
        write!(out, "<fractions>{}</fractions>", fraction_text(ticks)).ok();
    }
    write!(out, "</location></{tag}>").ok();
    out
}

/// The voices of a measure in the order `guitarpro` writes them (by voice
/// id, a later duplicate id replacing an earlier one - see `build_track`).
fn ordered_voices(measure: Option<&Measure>) -> Vec<&Voice> {
    let mut by_id: BTreeMap<u8, &Voice> = BTreeMap::new();
    for voice in measure.map(|m| m.voices.as_slice()).unwrap_or_default() {
        by_id.insert(voice.number.saturating_sub(1), voice);
    }
    by_id.into_values().collect()
}

/// Everything attached to an event, merged across a chord's notes (the
/// model stores marks per note; MuseScore per chord).
#[derive(Default)]
struct EventMarks<'a> {
    grace: bool,
    fermata: bool,
    articulations: Vec<&'a Articulation>,
    ornaments: Vec<&'a Ornament>,
    dynamic: Option<&'a Dynamic>,
    wedge: Option<Wedge>,
    chord_symbol: Option<&'a ChordSymbol>,
}

fn event_marks(event: &Event) -> EventMarks<'_> {
    let mut marks = EventMarks {
        chord_symbol: event.chord_symbol(),
        ..EventMarks::default()
    };
    let notes: &[Note] = match event {
        Event::Note(n) => std::slice::from_ref(n),
        Event::Chord(notes) => notes,
        Event::Rest(rest) => {
            marks.fermata = rest.fermata;
            &[]
        }
    };
    for note in notes {
        marks.grace |= note.grace;
        marks.fermata |= note.fermata;
        for a in &note.articulations {
            if !marks.articulations.contains(&a) {
                marks.articulations.push(a);
            }
        }
        for o in &note.ornaments {
            if !marks.ornaments.contains(&o) {
                marks.ornaments.push(o);
            }
        }
        marks.dynamic = marks.dynamic.or(note.dynamic.as_ref());
        marks.wedge = marks.wedge.or(note.wedge);
    }
    marks
}

fn beat_plan(event: &Event) -> BeatPlan {
    let marks = event_marks(event);
    let mut plan = BeatPlan::default();

    if let Some(symbol) = marks.chord_symbol {
        plan.before.push_str(&harmony_xml(symbol));
    }
    if let Some(dynamic) = marks.dynamic {
        plan.before.push_str(&dynamic_xml(dynamic));
    }
    if marks.fermata {
        plan.before
            .push_str("<Fermata><subtype>fermataAbove</subtype></Fermata>\n");
    }

    // Grace notes are only meaningful on a chord; the model doesn't record
    // a slash, so the unslashed appoggiatura is used.
    if marks.grace && !matches!(event, Event::Rest(_)) {
        plan.inside.push_str("<appoggiatura/>\n");
    }
    for articulation in marks.articulations {
        match articulation_to_mscx(articulation) {
            ArticulationMark::Symbol(symbol) => writeln!(
                plan.inside,
                "<Articulation><subtype>{}</subtype></Articulation>",
                xml_escape(&symbol)
            )
            .ok(),
            ArticulationMark::ChordLine(kind) => writeln!(
                plan.inside,
                "<ChordLine><subtype>{kind}</subtype></ChordLine>"
            )
            .ok(),
            ArticulationMark::Breath(symbol) => {
                writeln!(plan.after, "<Breath><symbol>{symbol}</symbol></Breath>").ok()
            }
        };
    }
    for ornament in marks.ornaments {
        if let Ornament::Tremolo(strokes) = ornament {
            writeln!(
                plan.inside,
                "<Tremolo><subtype>{}</subtype></Tremolo>",
                tremolo_subtype(*strokes)
            )
            .ok();
        } else if let Some(symbol) = ornament_symbol(ornament) {
            writeln!(
                plan.inside,
                "<Ornament><subtype>{}</subtype></Ornament>",
                xml_escape(symbol)
            )
            .ok();
        }
    }
    if matches!(event, Event::Rest(_)) {
        plan.inside.clear();
    }
    plan
}

fn harmony_xml(symbol: &ChordSymbol) -> String {
    if symbol.kind == ChordKind::NoChord {
        return "<Harmony><name>N.C.</name></Harmony>\n".to_string();
    }
    let mut out = format!("<Harmony><root>{}</root>", tpc_from_root(symbol.root));
    let suffix = symbol.kind.suffix();
    if !suffix.is_empty() {
        write!(out, "<name>{}</name>", xml_escape(suffix)).ok();
    }
    if let Some(bass) = symbol.bass {
        write!(out, "<base>{}</base>", tpc_from_root(bass)).ok();
    }
    out.push_str("</Harmony>\n");
    out
}

fn dynamic_xml(dynamic: &Dynamic) -> String {
    let name = dynamic_name(dynamic);
    if is_mscx_dynamic_subtype(name) {
        match dynamic_velocity(name) {
            Some(velocity) => format!(
                "<Dynamic><subtype>{name}</subtype><velocity>{velocity}</velocity></Dynamic>\n"
            ),
            None => format!("<Dynamic><subtype>{name}</subtype></Dynamic>\n"),
        }
    } else {
        format!(
            "<Dynamic><subtype>other-dynamics</subtype><text>{}</text></Dynamic>\n",
            xml_escape(name)
        )
    }
}

fn barline_xml(measure: Option<&Measure>) -> String {
    let Some(MscxBarline {
        subtype,
        span,
        visible,
    }) = measure.and_then(|m| barline_to_mscx(m.barline.kind))
    else {
        return String::new();
    };
    let mut out = format!("<BarLine><subtype>{subtype}</subtype>");
    if let Some((from, to)) = span {
        write!(out, "<spanFrom>{from}</spanFrom><spanTo>{to}</spanTo>").ok();
    }
    if !visible {
        out.push_str("<visible>0</visible>");
    }
    out.push_str("</BarLine>\n");
    out
}

fn navigation_xml(measure: Option<&Measure>) -> String {
    let marks = measure.map(|m| m.navigation.as_slice()).unwrap_or_default();
    marks
        .iter()
        .map(|&navigation| match navigation_to_mscx(navigation) {
            NavigationMark::Marker { label, text } => {
                format!("<Marker><text>{text}</text><label>{label}</label></Marker>\n")
            }
            NavigationMark::Jump {
                text,
                jump_to,
                play_until,
                continue_at,
            } => format!(
                "<Jump><text>{text}</text><jumpTo>{jump_to}</jumpTo><playUntil>{play_until}</playUntil><continueAt>{continue_at}</continueAt></Jump>\n"
            ),
        })
        .collect()
}

fn build_plan(score: &Score, measure_count: usize) -> Plan {
    let mut plan = Plan::default();
    for part in &score.parts {
        let measures = (0..measure_count)
            .map(|index| {
                let measure = part.measures.get(index);
                let voices = ordered_voices(measure);
                // `guitarpro` writes a single measure-rest voice for a
                // measure with no voices, which still gets the barline.
                let mut voice_plans: Vec<VoicePlan> = voices
                    .iter()
                    .map(|voice| VoicePlan {
                        beats: voice.events.iter().map(beat_plan).collect(),
                        ..VoicePlan::default()
                    })
                    .collect();
                if voice_plans.is_empty() {
                    voice_plans.push(VoicePlan::default());
                }
                voice_plans[0].tail.push_str(&barline_xml(measure));
                MeasurePlan {
                    head: String::new(),
                    voices: voice_plans,
                }
            })
            .collect();
        plan.staves.push(measures);
    }
    for (index, part) in score.parts.iter().enumerate() {
        plan_hairpins(part, &mut plan.staves[index]);
    }
    if let Some(first_staff) = plan.staves.first_mut() {
        plan_system_elements(score, first_staff);
    }
    plan
}

/// Pairs each hairpin start with the next `Stop` in the same voice (by voice
/// number, across barlines): the start element goes before the first
/// chord, the end element right after the last one. An unterminated
/// hairpin (or a stray `Stop`) is dropped.
fn plan_hairpins(part: &Part, staff: &mut [MeasurePlan]) {
    struct Open {
        location: (usize, usize, usize),
        ticks: i64,
        subtype: u8,
    }
    let mut open: HashMap<u8, Open> = HashMap::new();
    for (m, measure) in part.measures.iter().enumerate().take(staff.len()) {
        for (v, voice) in ordered_voices(Some(measure)).into_iter().enumerate() {
            let mut ticks = 0i64;
            for (b, event) in voice.events.iter().enumerate() {
                let start = ticks;
                ticks += event_ticks(event);
                match event_marks(event).wedge {
                    Some(Wedge::Stop) => {
                        let Some(o) = open.remove(&voice.number) else {
                            continue;
                        };
                        let (sm, sv, sb) = o.location;
                        let measures = (m - sm) as i64;
                        let fractions = ticks - o.ticks;
                        let voices = v as i64 - sv as i64;
                        staff[sm].voices[sv].beats[sb].before.push_str(&format!(
                            "<Spanner type=\"HairPin\"><HairPin><subtype>{}</subtype></HairPin>{}</Spanner>\n",
                            o.subtype,
                            location_xml("next", measures, fractions, voices)
                        ));
                        staff[m].voices[v].beats[b].after.push_str(&format!(
                            "<Spanner type=\"HairPin\">{}</Spanner>\n",
                            location_xml("prev", -measures, -fractions, -voices)
                        ));
                    }
                    Some(wedge) => {
                        if let Some(subtype) = hairpin_subtype(wedge) {
                            open.insert(
                                voice.number,
                                Open {
                                    location: (m, v, b),
                                    ticks: start,
                                    subtype,
                                },
                            );
                        }
                    }
                    None => {}
                }
            }
        }
    }
}

/// Jumps/markers and voltas are system-wide: written once, into the first
/// staff, from the first part that has each measure (like repeats).
fn plan_system_elements(score: &Score, staff: &mut [MeasurePlan]) {
    let mut time_signature = TimeSignature::default();
    // Length (in ticks) MuseScore will have read by the end of each
    // measure's first voice in the first staff.
    let mut lengths = Vec::with_capacity(staff.len());
    for index in 0..staff.len() {
        if let Some(ts) = shared_measure(score, index).and_then(|m| m.attributes.time_signature) {
            time_signature = ts;
        }
        let first_voice = score
            .parts
            .first()
            .and_then(|p| ordered_voices(p.measures.get(index)).first().copied())
            .filter(|v| !v.events.is_empty());
        lengths.push(match first_voice {
            Some(voice) => voice.events.iter().map(event_ticks).sum(),
            None => {
                WHOLE_TICKS * time_signature.beats as i64 / time_signature.beat_type.max(1) as i64
            }
        });
    }

    for index in 0..staff.len() {
        let measure = shared_measure(score, index);
        staff[index].head.push_str(&navigation_xml(measure));

        let Some(ending) = measure.and_then(|m| m.barline.ending.as_ref()) else {
            continue;
        };
        let end = (index + ending.measures.max(1) as usize - 1).min(staff.len() - 1);
        let closed = ending.closed;
        let numbers: Vec<String> = ending.numbers.iter().map(u8::to_string).collect();
        let numbers = numbers.join(", ");
        let measures = (end - index) as i64;
        staff[index].voices[0].head.push_str(&format!(
            "<Spanner type=\"Volta\"><Volta><endHookType>{}</endHookType><beginText>{numbers}.</beginText><endings>{numbers}</endings></Volta>{}</Spanner>\n",
            u8::from(closed),
            location_xml("next", measures, lengths[end], 0)
        ));
        staff[end].voices[0].tail.push_str(&format!(
            "<Spanner type=\"Volta\">{}</Spanner>\n",
            location_xml("prev", -measures, -lengths[end], 0)
        ));
    }
}
