//! Raw-MSCX passes for everything `guitarpro`'s mscz converter drops
//! (dynamics, hairpins, articulations, ornaments, fermatas, grace notes,
//! chord symbols, voltas, barline styles and jumps/markers).
//!
//! Reading: [`scan`] walks the same `<Score>/<Staff>/<Measure>/<voice>`
//! structure `guitarpro` parses and records, for every `<Chord>`/`<Rest>`
//! it turns into a beat, what else was attached to it - indexed exactly
//! like the converter's own output (staff = part, measure, voice position,
//! beat), so [`crate::read`] can overlay it.
//!
//! Writing: [`inject`] splices pre-rendered XML (a [`Plan`], built by
//! [`crate::write`]) into the MSCX `guitarpro` generated, at the matching
//! measure/voice/beat positions, copying everything else byte for byte.

use crate::convert::{
    ChordMark, articulation_from_breath, articulation_from_chord_line, barline_from_mscx,
    chord_marks_from_symbol, dynamic_from_name, navigation_from_jump, navigation_from_marker,
    root_from_tpc, tremolo_from_subtype, wedge_from_hairpin_subtype,
};
use quick_xml::events::{BytesStart, Event as XmlEvent};
use quick_xml::reader::Reader;
use scores_core::{
    Articulation, BarlineType, ChordKind, ChordSymbol, Dynamic, Ending, Navigation, Ornament,
    Result, ScoresError, Wedge,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Minimal DOM
// ---------------------------------------------------------------------------

/// Just enough of an XML element tree to interpret MSCX measures without a
/// hand-written state machine per element type.
#[derive(Debug, Default)]
struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    text: String,
    children: Vec<Element>,
}

impl Element {
    fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    fn has_child(&self, name: &str) -> bool {
        self.child(name).is_some()
    }

    /// Trimmed direct text of the first child named `name`.
    fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim())
    }

    /// First descendant (depth-first) named `name`.
    fn find(&self, name: &str) -> Option<&Element> {
        self.children.iter().find_map(|c| {
            if c.name == name {
                Some(c)
            } else {
                c.find(name)
            }
        })
    }

    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// All text in this subtree, e.g. `<text>To <b>Coda</b></text>`.
    fn all_text(&self) -> String {
        let mut out = self.text.clone();
        for child in &self.children {
            out.push_str(&child.all_text());
        }
        out
    }
}

fn xml_error(e: impl std::fmt::Display) -> ScoresError {
    ScoresError::Parse(format!("invalid MuseScore XML: {e}"))
}

fn start_element(start: &BytesStart<'_>) -> Result<Element> {
    let mut attributes = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(xml_error)?;
        let key = attribute.key.as_ref().to_string();
        let value = quick_xml::escape::unescape(&attribute.value)
            .map_err(xml_error)?
            .into_owned();
        attributes.push((key, value));
    }
    Ok(Element {
        name: start.name().as_ref().to_string(),
        attributes,
        ..Element::default()
    })
}

/// Parses a whole document into a synthetic root element.
fn parse_dom(xml: &str) -> Result<Element> {
    let mut reader = Reader::from_str(xml);
    let mut stack = vec![Element::default()];
    loop {
        match reader.read_event().map_err(xml_error)? {
            XmlEvent::Start(start) => stack.push(start_element(&start)?),
            XmlEvent::Empty(start) => {
                let element = start_element(&start)?;
                stack.last_mut().expect("root").children.push(element);
            }
            XmlEvent::End(_) => {
                if stack.len() > 1 {
                    let element = stack.pop().expect("checked len");
                    stack.last_mut().expect("root").children.push(element);
                }
            }
            XmlEvent::Text(text) => {
                let raw = text.into_inner();
                let unescaped = quick_xml::escape::unescape(&raw).map_err(xml_error)?;
                stack.last_mut().expect("root").text.push_str(&unescaped);
            }
            XmlEvent::CData(text) => {
                let raw = text.into_inner();
                stack.last_mut().expect("root").text.push_str(&raw);
            }
            XmlEvent::GeneralRef(reference) => {
                let resolved = match reference.resolve_char_ref().map_err(xml_error)? {
                    Some(c) => c.to_string(),
                    None => match reference.as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        _ => String::new(),
                    },
                };
                stack.last_mut().expect("root").text.push_str(&resolved);
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    // Unclosed elements (malformed input) are folded into their parents.
    while stack.len() > 1 {
        let element = stack.pop().expect("checked len");
        stack.last_mut().expect("root").children.push(element);
    }
    Ok(stack.pop().expect("root"))
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Extras for one score-level `<Staff>` (= one part).
#[derive(Debug, Default)]
pub(crate) struct StaffExtras {
    pub measures: Vec<MeasureExtras>,
}

#[derive(Debug, Default)]
pub(crate) struct MeasureExtras {
    /// This staff's end-of-measure barline style.
    pub barline: BarlineType,
    /// A volta bracket starting at this measure, spanning as many measures
    /// as the spanner covers. Voltas are system-wide, so only the first
    /// staff's count.
    pub ending: Option<Ending>,
    /// Every recognized `<Jump>`/`<Marker>`, in document order
    /// (system-wide, like `ending`).
    pub navigation: Vec<Navigation>,
    /// Per voice position, one entry per `<Chord>`/`<Rest>`.
    pub voices: Vec<Vec<BeatExtras>>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct BeatExtras {
    pub grace: bool,
    pub fermata: bool,
    pub articulations: Vec<Articulation>,
    pub ornaments: Vec<Ornament>,
    pub dynamic: Option<Dynamic>,
    pub wedge: Option<Wedge>,
    pub chord_symbol: Option<ChordSymbol>,
}

/// Where a beat lives: (measure index, voice position, beat index).
type BeatLocation = (usize, usize, usize);

/// Scans every score-level `<Staff>` - the same ones, in the same order,
/// that `guitarpro`'s parser turns into tracks.
pub(crate) fn scan(xml: &str) -> Result<Vec<StaffExtras>> {
    let root = parse_dom(xml)?;
    let mut staves = Vec::new();
    collect_staves(&root, false, &mut staves);
    Ok(staves)
}

fn collect_staves(element: &Element, in_score: bool, out: &mut Vec<StaffExtras>) {
    for child in &element.children {
        match child.name.as_str() {
            "Score" => collect_staves(child, true, out),
            "Staff" if in_score => out.push(scan_staff(child)),
            // A part's own `<Staff>` is only a staff definition.
            "Part" => {}
            _ => collect_staves(child, in_score, out),
        }
    }
}

fn scan_staff(staff: &Element) -> StaffExtras {
    let mut measures: Vec<MeasureExtras> = Vec::new();
    // Per voice position: the most recent beat, which a hairpin end or a
    // breath mark refers back to (possibly across a barline).
    let mut last_beat: HashMap<usize, BeatLocation> = HashMap::new();
    let mut open_volta: Option<(usize, Vec<u8>, bool)> = None;

    for measure_element in staff.children.iter().filter(|c| c.name == "Measure") {
        let m = measures.len();
        measures.push(MeasureExtras::default());
        let mut voice_position = 0;

        for child in &measure_element.children {
            match child.name.as_str() {
                "Jump" => {
                    let navigation = navigation_from_jump(
                        child.child_text("jumpTo").unwrap_or(""),
                        child.child_text("playUntil").unwrap_or(""),
                        &child
                            .child("text")
                            .map(Element::all_text)
                            .unwrap_or_default(),
                    );
                    measures[m].navigation.extend(navigation);
                }
                "Marker" => {
                    let navigation = navigation_from_marker(
                        child.child_text("label").unwrap_or(""),
                        &child
                            .child("text")
                            .map(Element::all_text)
                            .unwrap_or_default(),
                    );
                    measures[m].navigation.extend(navigation);
                }
                "voice" => {
                    scan_voice(
                        child,
                        &mut measures,
                        m,
                        voice_position,
                        &mut last_beat,
                        &mut open_volta,
                    );
                    voice_position += 1;
                }
                _ => {}
            }
        }
    }
    StaffExtras { measures }
}

fn beat_at(measures: &mut [MeasureExtras], (m, v, b): BeatLocation) -> Option<&mut BeatExtras> {
    measures.get_mut(m)?.voices.get_mut(v)?.get_mut(b)
}

fn scan_voice(
    voice: &Element,
    measures: &mut [MeasureExtras],
    m: usize,
    v: usize,
    last_beat: &mut HashMap<usize, BeatLocation>,
    open_volta: &mut Option<(usize, Vec<u8>, bool)>,
) {
    measures[m].voices.push(Vec::new());
    // Segment annotations (dynamics, harmony, fermatas, hairpin starts)
    // precede the chord/rest they belong to.
    let mut pending = BeatExtras::default();
    let mut seen_beat = false;
    let mut end_barline: Option<BarlineType> = None;

    for child in &voice.children {
        match child.name.as_str() {
            "Chord" | "Rest" => {
                let mut beat = std::mem::take(&mut pending);
                if child.name == "Chord" {
                    read_chord_marks(child, &mut beat);
                }
                let beats = &mut measures[m].voices[v];
                beats.push(beat);
                last_beat.insert(v, (m, v, beats.len() - 1));
                seen_beat = true;
                end_barline = None;
            }
            "Dynamic" => pending.dynamic = read_dynamic(child),
            "Harmony" => pending.chord_symbol = read_harmony(child),
            "Fermata" => pending.fermata = true,
            "Breath" => {
                let symbol = child
                    .child_text("symbol")
                    .or_else(|| child.child_text("subtype"))
                    .unwrap_or("");
                if let Some(beat) = last_beat.get(&v).and_then(|&loc| beat_at(measures, loc)) {
                    beat.articulations.push(articulation_from_breath(symbol));
                }
            }
            "Spanner" => match child.attribute("type") {
                Some("HairPin") => {
                    if let Some(hairpin) = child.child("HairPin") {
                        let subtype = hairpin.child_text("subtype").unwrap_or("0");
                        pending.wedge = Some(wedge_from_hairpin_subtype(subtype));
                    } else if child.has_child("prev") {
                        // The end marker sits at the hairpin's end tick,
                        // i.e. after the last chord it covers.
                        if let Some(beat) =
                            last_beat.get(&v).and_then(|&loc| beat_at(measures, loc))
                            && beat.wedge.is_none()
                        {
                            beat.wedge = Some(Wedge::Stop);
                        }
                    }
                }
                Some("Volta") => {
                    if let Some(volta) = child.child("Volta") {
                        let numbers = volta
                            .child_text("endings")
                            .unwrap_or("")
                            .split(',')
                            .filter_map(|n| n.trim().parse().ok())
                            .collect();
                        // MuseScore's default end hook is "none" (open).
                        let closed = volta.child_text("endHookType").is_some_and(|t| t != "0");
                        *open_volta = Some((m, numbers, closed));
                    } else if child.has_child("prev")
                        && let Some((start, numbers, closed)) = open_volta.take()
                    {
                        // At the start of a voice, the end tick is the end
                        // of the previous measure.
                        let end = if seen_beat { m } else { m.saturating_sub(1) }.max(start);
                        measures[start].ending = Some(Ending {
                            numbers,
                            measures: (end - start + 1) as u32,
                            closed,
                        });
                    }
                }
                _ => {}
            },
            "BarLine" => {
                let span_from = child.child_text("spanFrom").and_then(|t| t.parse().ok());
                let visible = child.child_text("visible") != Some("0");
                end_barline = Some(barline_from_mscx(
                    child.child_text("subtype").unwrap_or("normal"),
                    span_from,
                    visible,
                ));
            }
            _ => {}
        }
    }

    // Only a barline after the voice's last chord/rest is the measure's end
    // barline (not a start-repeat or mid-measure one).
    if let Some(kind) = end_barline
        && (v == 0 || measures[m].barline == BarlineType::Regular)
    {
        measures[m].barline = kind;
    }
}

fn read_chord_marks(chord: &Element, beat: &mut BeatExtras) {
    for child in &chord.children {
        match child.name.as_str() {
            "acciaccatura" | "appoggiatura" | "grace4" | "grace16" | "grace32" | "grace8after"
            | "grace16after" | "grace32after" => beat.grace = true,
            "Articulation" | "Ornament" => {
                let symbol = child.child_text("subtype").unwrap_or("");
                if symbol.is_empty() {
                    continue;
                }
                for mark in chord_marks_from_symbol(symbol) {
                    match mark {
                        ChordMark::Articulation(a) => beat.articulations.push(a),
                        ChordMark::Ornament(o) => beat.ornaments.push(o),
                        ChordMark::Fermata => beat.fermata = true,
                    }
                }
            }
            "Tremolo" | "TremoloSingleChord" => {
                if let Some(marks) = child.child_text("subtype").and_then(tremolo_from_subtype) {
                    beat.ornaments.push(Ornament::Tremolo(marks));
                }
            }
            "ChordLine" => {
                if let Some(a) = child
                    .child_text("subtype")
                    .and_then(articulation_from_chord_line)
                {
                    beat.articulations.push(a);
                }
            }
            _ => {}
        }
    }
}

fn read_dynamic(element: &Element) -> Option<Dynamic> {
    let subtype = element.child_text("subtype").unwrap_or("other-dynamics");
    if subtype != "other-dynamics" {
        return Some(dynamic_from_name(subtype));
    }
    let text = element.child("text")?.all_text();
    let text = text.trim();
    (!text.is_empty()).then(|| dynamic_from_name(text))
}

/// `<root>`/`<base>` are tonal pitch classes and `<name>` the suffix text;
/// they're searched anywhere in the element since newer MuseScore versions
/// nest them one level deeper. Roman-numeral/Nashville harmony is skipped.
fn read_harmony(element: &Element) -> Option<ChordSymbol> {
    if element.child_text("harmonyType").is_some_and(|t| t != "0") {
        return None;
    }
    let tpc = |name: &str| {
        element
            .find(name)
            .and_then(|e| e.text.trim().parse::<i32>().ok())
            .and_then(root_from_tpc)
    };
    let name = element
        .find("name")
        .map(|e| e.all_text())
        .unwrap_or_default();
    let name = name.trim();
    match tpc("root") {
        Some(root) => Some(ChordSymbol {
            root,
            kind: ChordKind::from_suffix(name),
            bass: tpc("base"),
        }),
        None => ChordSymbol::parse(name).filter(|c| c.kind == ChordKind::NoChord),
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Pre-rendered XML to splice in, indexed `[staff][measure]`.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    pub staves: Vec<Vec<MeasurePlan>>,
}

#[derive(Debug, Default)]
pub(crate) struct MeasurePlan {
    /// Right after `<Measure>` (jumps/markers).
    pub head: String,
    pub voices: Vec<VoicePlan>,
}

#[derive(Debug, Default)]
pub(crate) struct VoicePlan {
    /// Right after `<voice>` (volta starts).
    pub head: String,
    pub beats: Vec<BeatPlan>,
    /// Right before `</voice>` (barline, volta ends).
    pub tail: String,
}

#[derive(Debug, Default)]
pub(crate) struct BeatPlan {
    /// Before the `<Chord>`/`<Rest>` (segment annotations, hairpin starts).
    pub before: String,
    /// Right after the `<Chord>` start tag (grace flag, articulations...).
    pub inside: String,
    /// After the chord/rest (breath marks, hairpin ends).
    pub after: String,
}

impl Plan {
    fn measure(&self, staff: usize, measure: usize) -> Option<&MeasurePlan> {
        self.staves.get(staff)?.get(measure)
    }

    fn voice(&self, staff: usize, measure: usize, voice: usize) -> Option<&VoicePlan> {
        self.measure(staff, measure)?.voices.get(voice)
    }

    fn beat(&self, staff: usize, measure: usize, voice: usize, beat: usize) -> Option<&BeatPlan> {
        self.voice(staff, measure, voice)?.beats.get(beat)
    }
}

/// Splices `plan` into MSCX generated by `guitarpro`'s writer.
pub(crate) fn inject(xml: &str, plan: &Plan) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut out = String::with_capacity(xml.len() * 2);
    let mut copied = 0usize;
    let mut insert = |out: &mut String, at: usize, text: &str| {
        if !text.is_empty() {
            out.push_str(&xml[copied..at]);
            out.push_str(text);
            copied = at;
        }
    };

    let mut score_depth = 0u32;
    let mut part_depth = 0u32;
    let mut staff: Option<usize> = None;
    let mut next_staff = 0usize;
    let (mut measure, mut voice, mut beat) = (0usize, 0usize, 0usize);
    let mut in_voice = false;
    let mut in_beat = false;

    loop {
        let before = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| ScoresError::Write(format!("generated MSCX is malformed: {e}")))?;
        let after = reader.buffer_position() as usize;
        match event {
            XmlEvent::Start(start) => match start.name().as_ref() {
                "Score" => score_depth += 1,
                "Part" => part_depth += 1,
                "Staff" if score_depth > 0 && part_depth == 0 => {
                    staff = Some(next_staff);
                    next_staff += 1;
                    measure = 0;
                }
                "Measure" => {
                    if let Some(s) = staff {
                        voice = 0;
                        let head = plan.measure(s, measure).map(|p| p.head.as_str());
                        insert(&mut out, after, head.unwrap_or(""));
                    }
                }
                "voice" => {
                    if let Some(s) = staff {
                        in_voice = true;
                        beat = 0;
                        let head = plan.voice(s, measure, voice).map(|p| p.head.as_str());
                        insert(&mut out, after, head.unwrap_or(""));
                    }
                }
                "Chord" | "Rest" if in_voice && !in_beat => {
                    if let Some(s) = staff {
                        in_beat = true;
                        if let Some(p) = plan.beat(s, measure, voice, beat) {
                            insert(&mut out, before, &p.before);
                            if start.name().as_ref() == "Chord" {
                                insert(&mut out, after, &p.inside);
                            }
                        }
                    }
                }
                _ => {}
            },
            XmlEvent::End(end) => match end.name().as_ref() {
                "Score" => score_depth = score_depth.saturating_sub(1),
                "Part" => part_depth = part_depth.saturating_sub(1),
                "Staff" if part_depth == 0 => staff = None,
                "Measure" if staff.is_some() => measure += 1,
                "voice" => {
                    if let Some(s) = staff {
                        let tail = plan.voice(s, measure, voice).map(|p| p.tail.as_str());
                        insert(&mut out, before, tail.unwrap_or(""));
                        voice += 1;
                        in_voice = false;
                    }
                }
                "Chord" | "Rest" if in_beat => {
                    if let Some(s) = staff {
                        if let Some(p) = plan.beat(s, measure, voice, beat) {
                            insert(&mut out, after, &p.after);
                        }
                        beat += 1;
                        in_beat = false;
                    }
                }
                _ => {}
            },
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    out.push_str(&xml[copied..]);
    Ok(out)
}

/// Escapes text for element content.
pub(crate) fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}
