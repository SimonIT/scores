use crate::convert::{
    accid_ges_to_alter, attr_to_key_sig, attr_to_mode, attr_to_tie, code_to_articulation,
    dur_to_note_value, form_to_wedge, generic_kind_to_ornament, pname_to_step,
    rend_to_barline_type, repeat_mark_to_navigation, shape_line_to_clef, text_to_dynamic,
    type_to_navigation,
};
use quick_xml::events::{BytesStart, Event as XmlEvent};
use quick_xml::reader::Reader;
use scores_core::{
    Articulation, Barline, ChordSymbol, Clef, Duration, Dynamic, Ending, Event, KeySignature,
    Measure, MeasureAttributes, Navigation, Note, Ornament, Part, Pitch, Rest, Result, Score,
    ScoresError, TimeSignature, Tuplet, Voice, Wedge,
};
use std::collections::HashMap;

pub fn read(data: &[u8]) -> Result<Score> {
    let text = std::str::from_utf8(data)
        .map_err(|e| ScoresError::Parse(format!("input is not valid utf-8: {e}")))?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);

    let mut state = ReadState::default();

    loop {
        let event = reader
            .read_event()
            .map_err(|e| ScoresError::Parse(format!("invalid MEI XML: {e}")))?;
        match event {
            XmlEvent::Eof => break,
            XmlEvent::Start(e) => {
                let name = local_name(&e);
                state.handle_open(&name, &e);
            }
            XmlEvent::Empty(e) => {
                let name = local_name(&e);
                state.handle_open(&name, &e);
                state.handle_close(&name);
            }
            XmlEvent::End(e) => {
                let name = e.local_name().as_ref().to_string();
                state.handle_close(&name);
            }
            XmlEvent::Text(text) => {
                if let Some(target) = state.text_target {
                    let raw = text.into_inner();
                    if let Ok(unescaped) = quick_xml::escape::unescape(&raw) {
                        let buffer = match target {
                            "title" => &mut state.title,
                            "composer" => &mut state.composer,
                            _ => &mut state.control_text,
                        };
                        buffer.get_or_insert_with(String::new).push_str(&unescaped);
                    }
                }
            }
            _ => {}
        }
    }

    if state.parts.is_empty() {
        return Err(ScoresError::Parse(
            "MEI document has no staves with musical content".into(),
        ));
    }

    Ok(Score {
        title: state.title,
        composer: state.composer,
        parts: state.parts,
    })
}

fn local_name(e: &BytesStart) -> String {
    e.local_name().as_ref().to_string()
}

fn get_attr(e: &BytesStart, name: &str) -> Option<String> {
    e.try_get_attribute(name).ok().flatten().and_then(|a| {
        a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .ok()
            .map(|c| c.into_owned())
    })
}

/// `@artic` may hold a space-separated list (`artic="acc stacc"`).
fn artic_list(e: &BytesStart) -> Vec<Articulation> {
    get_attr(e, "artic")
        .map(|codes| codes.split_whitespace().map(code_to_articulation).collect())
        .unwrap_or_default()
}

#[derive(Clone, Copy, Default, PartialEq)]
struct StaffState {
    clef: Option<Clef>,
    key: Option<KeySignature>,
    time: Option<TimeSignature>,
}

/// A `<note>`/`<rest>` currently open (between its start and end tag, if it
/// has marking children) or just parsed (if self-closing).
struct PendingNote {
    id: Option<String>,
    pitch: Pitch,
    duration: Duration,
    tie: scores_core::Tie,
    grace: bool,
    fermata: bool,
    articulations: Vec<Articulation>,
    ornaments: Vec<Ornament>,
}

/// A `<chord>` being accumulated. Markings on the `<chord>` itself (rather
/// than on one of its notes) end up on the first note, like the model's
/// chord-level dynamics and chord symbols.
struct ChordAccum {
    id: Option<String>,
    dur: u32,
    dots: u8,
    grace: bool,
    fermata: bool,
    articulations: Vec<Articulation>,
    ornaments: Vec<Ornament>,
    notes: Vec<Note>,
}

/// Where a `<note>`/`<rest>`/`<chord>` ended up in the current measure, so
/// real-MEI control events (`@startid`/`@tstamp`) can be resolved against it
/// once the whole `<measure>` is read.
#[derive(Clone, Copy)]
struct EventLoc {
    staff: u32,
    voice_index: usize,
    event_index: usize,
    /// A specific note within a chord (the chord's own id leaves this unset).
    note_index: Option<usize>,
}

enum ControlTarget {
    Id(String),
    Tstamp { staff: u32, tstamp: f64 },
}

/// A marking that attaches to an event: either one written the way this
/// plugin writes it (a sibling right before the event, or nested inside it)
/// or a real-MEI control event pointing at it.
enum Control {
    Dynamic(Dynamic),
    Wedge(Wedge),
    ChordSymbol(ChordSymbol),
    Fermata,
    Ornament(Ornament),
}

#[derive(Clone, Copy, PartialEq)]
enum TextControl {
    Dynam,
    Harm,
    RepeatMark,
    Dir,
}

/// A text-bearing control element (`<dynam>`, `<harm>`, `<repeatMark>`,
/// `<dir>`) whose text is still being collected.
struct OpenControl {
    kind: TextControl,
    target: Option<ControlTarget>,
    func: Option<String>,
}

/// An `<ending>` currently open: it may wrap several measures, so it is only
/// applied (to its first measure, spanning all of them) once its end tag is
/// seen.
struct OpenEnding {
    numbers: Vec<u8>,
    open_end: bool,
    /// Each part's measure count when the ending began.
    start_lens: Vec<usize>,
}

#[derive(Default)]
pub(crate) struct ReadState {
    title: Option<String>,
    composer: Option<String>,
    text_target: Option<&'static str>,
    control_text: Option<String>,
    open_control: Option<OpenControl>,

    staff_defs: HashMap<u32, StaffState>,
    labels: HashMap<u32, String>,

    staff_state: HashMap<u32, StaffState>,
    parts: Vec<Part>,

    current_measure_number: u32,
    current_barline: Barline,
    current_navigation: Vec<Navigation>,
    current_ending: Option<OpenEnding>,
    /// Set by `right="rptboth"`: the next measure starts a repeat.
    repeat_start_next: bool,

    current_staff_n: Option<u32>,
    current_layer_n: Option<u8>,
    current_events: Vec<Event>,
    current_voices: Vec<Voice>,
    staff_before: StaffState,
    /// Staves (parts) that received a measure this `<measure>` element, so
    /// navigation and control events (written after all staves) can be
    /// attached to exactly those parts' just-pushed measure once the whole
    /// element is known.
    staves_this_measure: Vec<u32>,

    /// Time position within the current layer, in quarter notes.
    layer_cursor: f64,
    measure_ids: HashMap<String, EventLoc>,
    measure_onsets: Vec<(EventLoc, f64)>,
    deferred_controls: Vec<(ControlTarget, Control)>,

    pending_note: Option<PendingNote>,
    pending_rest: Option<(Option<String>, Duration, bool)>,
    pending_dynamic: Option<Dynamic>,
    pending_wedge: Option<Wedge>,
    pending_chord_symbol: Option<ChordSymbol>,

    chord_accum: Option<ChordAccum>,
    tuplet_stack: Vec<Tuplet>,
}

/// `@startid` (`#id`) wins over `@tstamp`; neither means the element is
/// positioned the way this plugin writes it (see module docs).
fn control_target(e: &BytesStart) -> Option<ControlTarget> {
    if let Some(id) = get_attr(e, "startid") {
        return Some(ControlTarget::Id(id.trim_start_matches('#').to_string()));
    }
    let tstamp = get_attr(e, "tstamp")?.trim().parse().ok()?;
    let staff = get_attr(e, "staff")
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(1);
    Some(ControlTarget::Tstamp { staff, tstamp })
}

impl ReadState {
    fn ensure_part(&mut self, n: u32) {
        while (self.parts.len() as u32) < n {
            let index = self.parts.len() as u32 + 1;
            self.parts.push(Part {
                id: format!("P{index}"),
                name: self.labels.get(&index).cloned(),
                measures: Vec::new(),
            });
        }
    }

    fn open_text_control(&mut self, kind: TextControl, e: &BytesStart) {
        self.text_target = Some("control");
        self.control_text = None;
        self.open_control = Some(OpenControl {
            kind,
            target: control_target(e),
            func: get_attr(e, "func"),
        });
    }

    /// Attaches a fermata/ornament found outside any `@startid`: nested in a
    /// `<note>`/`<rest>`/`<chord>` as this plugin writes them.
    fn attach_nested(&mut self, control: Control, e: &BytesStart) {
        if let Some(note) = &mut self.pending_note {
            match control {
                Control::Fermata => note.fermata = true,
                Control::Ornament(o) => note.ornaments.push(o),
                _ => {}
            }
        } else if let Some(chord) = &mut self.chord_accum {
            match control {
                Control::Fermata => chord.fermata = true,
                Control::Ornament(o) => chord.ornaments.push(o),
                _ => {}
            }
        } else if let Some((_, _, fermata)) = &mut self.pending_rest {
            if matches!(control, Control::Fermata) {
                *fermata = true;
            }
        } else if let Some(target) = control_target(e) {
            self.deferred_controls.push((target, control));
        }
    }

    fn handle_open(&mut self, name: &str, e: &BytesStart) {
        match name {
            "title" => self.text_target = Some("title"),
            "composer" => self.text_target = Some("composer"),
            "dynam" => self.open_text_control(TextControl::Dynam, e),
            "harm" => self.open_text_control(TextControl::Harm, e),
            "repeatMark" => self.open_text_control(TextControl::RepeatMark, e),
            "dir" => self.open_text_control(TextControl::Dir, e),
            "hairpin" => {
                let wedge = get_attr(e, "form").and_then(|f| form_to_wedge(&f));
                match control_target(e) {
                    Some(target) => {
                        let staff = match &target {
                            ControlTarget::Tstamp { staff, .. } => *staff,
                            ControlTarget::Id(_) => 1,
                        };
                        if let Some(wedge) = wedge {
                            self.deferred_controls.push((target, Control::Wedge(wedge)));
                        }
                        // Only a hairpin ending in the same measure can be resolved.
                        let end = if let Some(id) = get_attr(e, "endid") {
                            Some(ControlTarget::Id(id.trim_start_matches('#').to_string()))
                        } else {
                            get_attr(e, "tstamp2")
                                .and_then(|t| {
                                    let (measures, beat) = t.split_once("m+")?;
                                    (measures.trim() == "0").then(|| beat.trim().parse().ok())?
                                })
                                .map(|tstamp| ControlTarget::Tstamp { staff, tstamp })
                        };
                        if let Some(end) = end {
                            self.deferred_controls
                                .push((end, Control::Wedge(Wedge::Stop)));
                        }
                    }
                    None => {
                        if let Some(wedge) = wedge {
                            self.pending_wedge = Some(wedge);
                        }
                    }
                }
            }
            "staffDef" => {
                let Some(n) = get_attr(e, "n").and_then(|s| s.parse().ok()) else {
                    return;
                };
                if let Some(label) = get_attr(e, "label") {
                    self.labels.insert(n, label);
                }
                let mut staff = StaffState::default();
                if let (Some(shape), Some(line)) =
                    (get_attr(e, "clef.shape"), get_attr(e, "clef.line"))
                {
                    staff.clef = Some(shape_line_to_clef(&shape, line.parse().unwrap_or(0)));
                }
                if let Some(sig) = get_attr(e, "key.sig") {
                    let mode = get_attr(e, "key.mode")
                        .map(|m| attr_to_mode(&m))
                        .unwrap_or(scores_core::Mode::Major);
                    staff.key = Some(KeySignature {
                        fifths: attr_to_key_sig(&sig),
                        mode,
                    });
                }
                if let (Some(count), Some(unit)) =
                    (get_attr(e, "meter.count"), get_attr(e, "meter.unit"))
                    && let (Ok(beats), Ok(beat_type)) = (count.parse(), unit.parse())
                {
                    staff.time = Some(TimeSignature { beats, beat_type });
                }
                self.staff_defs.insert(n, staff);
            }
            "ending" => {
                let numbers = get_attr(e, "n")
                    .map(|n| n.split(',').filter_map(|p| p.trim().parse().ok()).collect())
                    .unwrap_or_default();
                // Older versions of this plugin marked a one-measure open
                // ending with a non-standard `@type="discontinue"`.
                let open_end = get_attr(e, "lendsym").as_deref() == Some("none")
                    || get_attr(e, "type").as_deref() == Some("discontinue");
                self.current_ending = Some(OpenEnding {
                    numbers,
                    open_end,
                    start_lens: self.parts.iter().map(|p| p.measures.len()).collect(),
                });
            }
            "measure" => {
                self.current_measure_number = get_attr(e, "n")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(self.current_measure_number + 1);
                self.current_barline = Barline::default();
                if std::mem::take(&mut self.repeat_start_next) {
                    self.current_barline.repeat_start = true;
                }
                match get_attr(e, "left").as_deref() {
                    Some("rptstart") => self.current_barline.repeat_start = true,
                    Some("rptboth") => {
                        // Also closes the repeat ending at the previous measure.
                        self.current_barline.repeat_start = true;
                        for part in &mut self.parts {
                            if let Some(prev) = part.measures.last_mut() {
                                prev.barline.repeat_end.get_or_insert(2);
                            }
                        }
                    }
                    _ => {}
                }
                match get_attr(e, "right").as_deref() {
                    Some(right @ ("rptend" | "rptboth")) => {
                        let times = get_attr(e, "repeat.count")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(2);
                        self.current_barline.repeat_end = Some(times);
                        self.repeat_start_next = right == "rptboth";
                    }
                    Some(rend) => self.current_barline.kind = rend_to_barline_type(rend),
                    None => {}
                }
                self.current_navigation.clear();
                self.staves_this_measure.clear();
                self.measure_ids.clear();
                self.measure_onsets.clear();
                self.deferred_controls.clear();
            }
            "navigation" => {
                if let Some(navigation) = get_attr(e, "type").and_then(|t| type_to_navigation(&t)) {
                    self.current_navigation.push(navigation);
                }
            }
            "staff" => {
                let Some(n) = get_attr(e, "n").and_then(|s| s.parse().ok()) else {
                    return;
                };
                self.current_staff_n = Some(n);
                self.current_voices = Vec::new();
                self.staff_state
                    .entry(n)
                    .or_insert_with(|| self.staff_defs.get(&n).copied().unwrap_or_default());
                self.staff_before = self.staff_state[&n];
                self.ensure_part(n);
            }
            "layer" => {
                self.current_layer_n =
                    Some(get_attr(e, "n").and_then(|s| s.parse().ok()).unwrap_or(1));
                self.current_events = Vec::new();
                self.layer_cursor = 0.0;
            }
            "clef" => {
                if let (Some(n), Some(shape)) = (self.current_staff_n, get_attr(e, "shape")) {
                    let line = get_attr(e, "line")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    self.staff_state.entry(n).or_default().clef =
                        Some(shape_line_to_clef(&shape, line));
                }
            }
            "keySig" => {
                if let (Some(n), Some(sig)) = (self.current_staff_n, get_attr(e, "sig")) {
                    let mode = get_attr(e, "mode")
                        .map(|m| attr_to_mode(&m))
                        .unwrap_or(scores_core::Mode::Major);
                    self.staff_state.entry(n).or_default().key = Some(KeySignature {
                        fifths: attr_to_key_sig(&sig),
                        mode,
                    });
                }
            }
            "meterSig" => {
                if let (Some(n), Some(count), Some(unit)) = (
                    self.current_staff_n,
                    get_attr(e, "count"),
                    get_attr(e, "unit"),
                ) && let (Ok(beats), Ok(beat_type)) = (count.parse(), unit.parse())
                {
                    self.staff_state.entry(n).or_default().time =
                        Some(TimeSignature { beats, beat_type });
                }
            }
            "tuplet" => {
                let actual = get_attr(e, "num").and_then(|s| s.parse().ok()).unwrap_or(3);
                let normal = get_attr(e, "numbase")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(2);
                self.tuplet_stack.push(Tuplet { actual, normal });
            }
            "chord" => {
                self.chord_accum = Some(ChordAccum {
                    id: get_attr(e, "xml:id"),
                    dur: get_attr(e, "dur").and_then(|s| s.parse().ok()).unwrap_or(4),
                    dots: get_attr(e, "dots")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    grace: get_attr(e, "grace").is_some(),
                    fermata: get_attr(e, "fermata").is_some(),
                    articulations: artic_list(e),
                    ornaments: Vec::new(),
                    notes: Vec::new(),
                });
            }
            "note" => {
                let (dur, dots, chord_grace) = match &self.chord_accum {
                    Some(chord) => (chord.dur, chord.dots, chord.grace),
                    None => (
                        get_attr(e, "dur").and_then(|s| s.parse().ok()).unwrap_or(4),
                        get_attr(e, "dots")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        false,
                    ),
                };
                let pname = get_attr(e, "pname").unwrap_or_else(|| "c".to_string());
                let octave = get_attr(e, "oct").and_then(|s| s.parse().ok()).unwrap_or(4);
                let alter = get_attr(e, "accid.ges")
                    .map(|a| accid_ges_to_alter(&a))
                    .unwrap_or(0);
                let tie = get_attr(e, "tie")
                    .map(|t| attr_to_tie(&t))
                    .unwrap_or(scores_core::Tie::None);

                self.pending_note = Some(PendingNote {
                    id: get_attr(e, "xml:id"),
                    pitch: Pitch::new(pname_to_step(&pname), alter, octave),
                    duration: Duration {
                        base: dur_to_note_value(dur),
                        dots,
                        tuplet: None,
                    },
                    tie,
                    grace: chord_grace || get_attr(e, "grace").is_some(),
                    fermata: get_attr(e, "fermata").is_some(),
                    articulations: artic_list(e),
                    ornaments: Vec::new(),
                });
            }
            "rest" => {
                let dur = get_attr(e, "dur").and_then(|s| s.parse().ok()).unwrap_or(4);
                let dots = get_attr(e, "dots")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                self.pending_rest = Some((
                    get_attr(e, "xml:id"),
                    Duration {
                        base: dur_to_note_value(dur),
                        dots,
                        tuplet: None,
                    },
                    get_attr(e, "fermata").is_some(),
                ));
            }
            "artic" => {
                let articulations = artic_list(e);
                if let Some(note) = &mut self.pending_note {
                    note.articulations.extend(articulations);
                } else if let Some(chord) = &mut self.chord_accum {
                    chord.articulations.extend(articulations);
                }
            }
            "trill" => self.attach_nested(Control::Ornament(Ornament::TrillMark), e),
            "mordent" => {
                let upper = get_attr(e, "form").as_deref() == Some("upper");
                let ornament = if upper {
                    Ornament::InvertedMordent
                } else {
                    Ornament::Mordent
                };
                self.attach_nested(Control::Ornament(ornament), e);
            }
            "turn" => {
                let lower = get_attr(e, "form").as_deref() == Some("lower");
                let ornament = if lower {
                    Ornament::InvertedTurn
                } else {
                    Ornament::Turn
                };
                self.attach_nested(Control::Ornament(ornament), e);
            }
            "ornam" => {
                let kind = get_attr(e, "type").unwrap_or_default();
                let count = get_attr(e, "count").and_then(|s| s.parse().ok());
                let ornament = generic_kind_to_ornament(&kind, count);
                self.attach_nested(Control::Ornament(ornament), e);
            }
            "fermata" => self.attach_nested(Control::Fermata, e),
            _ => {}
        }
    }

    fn handle_close(&mut self, name: &str) {
        match name {
            "title" | "composer" => self.text_target = None,
            "dynam" | "harm" | "repeatMark" | "dir" => self.close_text_control(),
            "ending" => self.close_ending(),
            "tuplet" => {
                self.tuplet_stack.pop();
            }
            "note" => {
                let Some(pending) = self.pending_note.take() else {
                    return;
                };
                let mut note = Note::new(pending.pitch, pending.duration, pending.tie);
                note.grace = pending.grace;
                note.fermata = pending.fermata;
                note.articulations = pending.articulations;
                note.ornaments = pending.ornaments;

                match &mut self.chord_accum {
                    Some(chord) => {
                        if let (Some(id), Some(staff)) = (pending.id, self.current_staff_n) {
                            let loc = EventLoc {
                                staff,
                                voice_index: self.current_voices.len(),
                                event_index: self.current_events.len(),
                                note_index: Some(chord.notes.len()),
                            };
                            self.measure_ids.insert(id, loc);
                        }
                        chord.notes.push(note);
                    }
                    None => {
                        note.dynamic = self.pending_dynamic.take();
                        note.wedge = self.pending_wedge.take();
                        note.chord_symbol = self.pending_chord_symbol.take();
                        self.push_event(Event::Note(note), pending.id);
                    }
                }
            }
            "rest" => {
                if let Some((id, duration, fermata)) = self.pending_rest.take() {
                    let event = Event::Rest(Rest {
                        duration,
                        fermata,
                        chord_symbol: self.pending_chord_symbol.take(),
                    });
                    self.push_event(event, id);
                }
            }
            "chord" => {
                if let Some(mut chord) = self.chord_accum.take() {
                    if let Some(first) = chord.notes.first_mut() {
                        first.fermata |= chord.fermata;
                        first.articulations.append(&mut chord.articulations);
                        first.ornaments.append(&mut chord.ornaments);
                        first.dynamic = self.pending_dynamic.take();
                        first.wedge = self.pending_wedge.take();
                        first.chord_symbol = self.pending_chord_symbol.take();
                    }
                    self.push_event(Event::Chord(chord.notes), chord.id);
                }
            }
            "layer" => {
                let number = self.current_layer_n.take().unwrap_or(1);
                self.current_voices.push(Voice {
                    number,
                    events: std::mem::take(&mut self.current_events),
                });
            }
            "staff" => {
                let Some(n) = self.current_staff_n.take() else {
                    return;
                };
                let after = self.staff_state.get(&n).copied().unwrap_or_default();
                let part = &mut self.parts[(n - 1) as usize];
                let changed = part.measures.is_empty() || after != self.staff_before;
                let attributes = if changed {
                    MeasureAttributes {
                        time_signature: after.time,
                        key_signature: after.key,
                        clef: after.clef,
                        tempo: None,
                    }
                } else {
                    MeasureAttributes::default()
                };
                let mut voices = std::mem::take(&mut self.current_voices);
                if voices.is_empty() {
                    voices.push(Voice {
                        number: 1,
                        events: Vec::new(),
                    });
                }
                part.measures.push(Measure {
                    number: self.current_measure_number,
                    attributes,
                    voices,
                    barline: self.current_barline.clone(),
                    navigation: Vec::new(),
                });
                self.staves_this_measure.push(n);
            }
            "measure" => {
                self.resolve_deferred_controls();
                for &n in &self.staves_this_measure {
                    if let Some(part) = self.parts.get_mut((n - 1) as usize)
                        && let Some(measure) = part.measures.last_mut()
                    {
                        measure.navigation.clone_from(&self.current_navigation);
                    }
                }
            }
            _ => {}
        }
    }

    fn close_text_control(&mut self) {
        self.text_target = None;
        let text = self.control_text.take().unwrap_or_default();
        let Some(open) = self.open_control.take() else {
            return;
        };
        let control = match open.kind {
            TextControl::Dynam if !text.trim().is_empty() => {
                Control::Dynamic(text_to_dynamic(text.trim()))
            }
            TextControl::Harm => match ChordSymbol::parse(&text) {
                Some(symbol) => Control::ChordSymbol(symbol),
                None => return,
            },
            TextControl::RepeatMark | TextControl::Dir => {
                let func = if open.kind == TextControl::RepeatMark {
                    open.func.as_deref()
                } else {
                    None
                };
                if let Some(navigation) = repeat_mark_to_navigation(func, &text) {
                    self.current_navigation.push(navigation);
                }
                return;
            }
            _ => return,
        };
        match open.target {
            Some(target) => self.deferred_controls.push((target, control)),
            None => match control {
                Control::Dynamic(d) => self.pending_dynamic = Some(d),
                Control::ChordSymbol(c) => self.pending_chord_symbol = Some(c),
                _ => {}
            },
        }
    }

    fn close_ending(&mut self) {
        let Some(ending) = self.current_ending.take() else {
            return;
        };
        for (index, part) in self.parts.iter_mut().enumerate() {
            let start = ending.start_lens.get(index).copied().unwrap_or(0);
            let count = part.measures.len().saturating_sub(start);
            if count == 0 {
                continue;
            }
            part.measures[start].barline.ending = Some(Ending {
                numbers: ending.numbers.clone(),
                measures: count as u32,
                closed: !ending.open_end,
            });
        }
    }

    fn push_event(&mut self, mut event: Event, id: Option<String>) {
        if let Some(&tuplet) = self.tuplet_stack.last() {
            match &mut event {
                Event::Note(n) => n.duration.tuplet = Some(tuplet),
                Event::Rest(r) => r.duration.tuplet = Some(tuplet),
                Event::Chord(notes) => {
                    for n in notes.iter_mut() {
                        n.duration.tuplet = Some(tuplet);
                    }
                }
            }
        }
        if let Some(staff) = self.current_staff_n {
            let loc = EventLoc {
                staff,
                voice_index: self.current_voices.len(),
                event_index: self.current_events.len(),
                note_index: None,
            };
            if let Some(id) = id {
                self.measure_ids.insert(id, loc);
            }
            let grace = match &event {
                Event::Note(n) => n.grace,
                Event::Chord(notes) => notes.first().is_some_and(|n| n.grace),
                Event::Rest(_) => false,
            };
            if !grace {
                // MEI's @tstamp counts beats of the meter's unit, from 1.
                let unit = self.staff_state.get(&staff).and_then(|s| s.time);
                let unit = unit.map_or(4, |t| t.beat_type) as f64;
                let tstamp = 1.0 + self.layer_cursor * unit / 4.0;
                self.measure_onsets.push((loc, tstamp));
                self.layer_cursor += event.duration().quarter_length();
            }
        }
        self.current_events.push(event);
    }

    /// For `@tstamp`: the event starting exactly there (lowest layer first),
    /// else the latest one starting before it.
    fn find_onset(&self, staff: u32, tstamp: f64) -> Option<EventLoc> {
        let on_staff = || self.measure_onsets.iter().filter(|(l, _)| l.staff == staff);
        on_staff()
            .filter(|(_, t)| (t - tstamp).abs() < 1e-3)
            .min_by_key(|(l, _)| l.voice_index)
            .or_else(|| {
                on_staff().filter(|(_, t)| *t <= tstamp).max_by(|a, b| {
                    a.1.total_cmp(&b.1)
                        .then(b.0.voice_index.cmp(&a.0.voice_index))
                })
            })
            .or_else(|| on_staff().next())
            .map(|(l, _)| *l)
    }

    fn resolve_deferred_controls(&mut self) {
        for (target, control) in std::mem::take(&mut self.deferred_controls) {
            let loc = match target {
                ControlTarget::Id(id) => self.measure_ids.get(&id).copied(),
                ControlTarget::Tstamp { staff, tstamp } => self.find_onset(staff, tstamp),
            };
            let Some(loc) = loc else { continue };
            if !self.staves_this_measure.contains(&loc.staff) {
                continue;
            }
            let event = self
                .parts
                .get_mut((loc.staff - 1) as usize)
                .and_then(|p| p.measures.last_mut())
                .and_then(|m| m.voices.get_mut(loc.voice_index))
                .and_then(|v| v.events.get_mut(loc.event_index));
            if let Some(event) = event {
                apply_control(event, loc.note_index, control);
            }
        }
    }
}

/// Dynamics, wedges and chord symbols on a chord always go on its first
/// note (the model's convention); fermatas/ornaments pointing at one
/// specific note of a chord stay on that note.
fn apply_control(event: &mut Event, note_index: Option<usize>, control: Control) {
    match event {
        Event::Rest(rest) => match control {
            Control::Fermata => rest.fermata = true,
            Control::ChordSymbol(c) => rest.chord_symbol = Some(c),
            _ => {}
        },
        Event::Note(_) | Event::Chord(_) => {
            let note = match event {
                Event::Note(n) => Some(n),
                Event::Chord(notes) => {
                    let index = match control {
                        Control::Fermata | Control::Ornament(_) => note_index.unwrap_or(0),
                        _ => 0,
                    };
                    notes.get_mut(index)
                }
                Event::Rest(_) => None,
            };
            let Some(note) = note else { return };
            match control {
                Control::Dynamic(d) => note.dynamic = Some(d),
                Control::Wedge(w) => note.wedge = Some(w),
                Control::ChordSymbol(c) => note.chord_symbol = Some(c),
                Control::Fermata => note.fermata = true,
                Control::Ornament(o) => note.ornaments.push(o),
            }
        }
    }
}
