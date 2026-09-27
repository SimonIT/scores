use crate::convert::{
    OrnamentElement, articulation_to_code, barline_type_to_rend, clef_to_shape_line,
    dynamic_to_text, key_sig_to_attr, mode_to_attr, navigation_at_measure_end,
    navigation_to_repeat_mark, note_value_to_dur, ornament_to_element, step_to_pname, tie_to_attr,
    wedge_to_form,
};
use quick_xml::events::{BytesDecl, BytesText};
use quick_xml::writer::Writer;
use scores_core::{
    ChordSymbol, Clef, Ending, Event, KeySignature, Measure, Navigation, Note, Part, Rest, Result,
    Score, ScoresError, TimeSignature,
};
use std::io;

pub fn write(score: &Score) -> Result<Vec<u8>> {
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    write_document(&mut writer, score)
        .map_err(|e| ScoresError::Write(format!("failed to serialize MEI: {e}")))?;
    Ok(writer.into_inner())
}

fn write_document(writer: &mut Writer<Vec<u8>>, score: &Score) -> io::Result<()> {
    writer.write_event(quick_xml::events::Event::Decl(BytesDecl::new(
        "1.0",
        Some("UTF-8"),
        None,
    )))?;
    writer
        .create_element("mei")
        .with_attribute(("xmlns", "http://www.music-encoding.org/ns/mei"))
        .with_attribute(("meiversion", "5.0"))
        .write_inner_content(|writer| {
            write_mei_head(writer, score)?;
            write_music(writer, score)?;
            Ok(())
        })?;
    Ok(())
}

fn write_mei_head(writer: &mut Writer<Vec<u8>>, score: &Score) -> io::Result<()> {
    if score.title.is_none() && score.composer.is_none() {
        return Ok(());
    }
    writer
        .create_element("meiHead")
        .write_inner_content(|writer| {
            writer
                .create_element("fileDesc")
                .write_inner_content(|writer| {
                    writer
                        .create_element("titleStmt")
                        .write_inner_content(|writer| {
                            if let Some(title) = &score.title {
                                writer
                                    .create_element("title")
                                    .write_text_content(BytesText::new(title))?;
                            }
                            if let Some(composer) = &score.composer {
                                writer
                                    .create_element("composer")
                                    .write_text_content(BytesText::new(composer))?;
                            }
                            Ok(())
                        })?;
                    Ok(())
                })?;
            Ok(())
        })?;
    Ok(())
}

fn write_music(writer: &mut Writer<Vec<u8>>, score: &Score) -> io::Result<()> {
    writer
        .create_element("music")
        .write_inner_content(|writer| {
            writer
                .create_element("body")
                .write_inner_content(|writer| {
                    writer
                        .create_element("mdiv")
                        .write_inner_content(|writer| {
                            writer
                                .create_element("score")
                                .write_inner_content(|writer| {
                                    write_score_def(writer, score)?;
                                    write_section(writer, score)?;
                                    Ok(())
                                })?;
                            Ok(())
                        })?;
                    Ok(())
                })?;
            Ok(())
        })?;
    Ok(())
}

fn write_score_def(writer: &mut Writer<Vec<u8>>, score: &Score) -> io::Result<()> {
    writer
        .create_element("scoreDef")
        .write_inner_content(|writer| {
            writer
                .create_element("staffGrp")
                .write_inner_content(|writer| {
                    for (index, part) in score.parts.iter().enumerate() {
                        write_staff_def(writer, index, part)?;
                    }
                    Ok(())
                })?;
            Ok(())
        })?;
    Ok(())
}

fn write_staff_def(writer: &mut Writer<Vec<u8>>, index: usize, part: &Part) -> io::Result<()> {
    let attrs = part
        .measures
        .first()
        .map(|m| m.attributes.clone())
        .unwrap_or_default();

    let mut pairs: Vec<(&str, String)> =
        vec![("n", (index + 1).to_string()), ("lines", "5".to_string())];
    if let Some(name) = &part.name {
        pairs.push(("label", name.clone()));
    }
    if let Some(clef) = attrs.clef {
        let (shape, line) = clef_to_shape_line(clef);
        pairs.push(("clef.shape", shape.to_string()));
        pairs.push(("clef.line", line.to_string()));
    }
    if let Some(key) = attrs.key_signature {
        pairs.push(("key.sig", key_sig_to_attr(key.fifths)));
        pairs.push(("key.mode", mode_to_attr(key.mode).to_string()));
    }
    if let Some(ts) = attrs.time_signature {
        pairs.push(("meter.count", ts.beats.to_string()));
        pairs.push(("meter.unit", ts.beat_type.to_string()));
    }

    writer
        .create_element("staffDef")
        .with_attributes(pairs.iter().map(|(k, v)| (*k, v.as_str())))
        .write_empty()?;
    Ok(())
}

fn write_section(writer: &mut Writer<Vec<u8>>, score: &Score) -> io::Result<()> {
    let measure_count = score
        .parts
        .iter()
        .map(|p| p.measures.len())
        .max()
        .unwrap_or(0);
    writer
        .create_element("section")
        .write_inner_content(|writer| {
            // Tracked only to place end-of-measure navigation marks' @tstamp.
            let mut beats = TimeSignature::default().beats;
            let mut measure_index = 0;
            while measure_index < measure_count {
                let Some(ending) =
                    shared_measure(score, measure_index).and_then(|m| m.barline.ending.clone())
                else {
                    write_measure(writer, score, measure_index, &mut beats)?;
                    measure_index += 1;
                    continue;
                };
                let last = ending_span_end(score, measure_index, measure_count, &ending);
                let numbers = ending
                    .numbers
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let lendsym = if ending.closed { "angledown" } else { "none" };
                let el = writer
                    .create_element("ending")
                    .with_attribute(("n", numbers.as_str()))
                    .with_attribute(("lendsym", lendsym));
                el.write_inner_content(|writer| {
                    for index in measure_index..=last {
                        write_measure(writer, score, index, &mut beats)?;
                    }
                    Ok(())
                })?;
                measure_index = last + 1;
            }
            Ok(())
        })?;
    Ok(())
}

/// A real MEI `<ending>` wraps every measure under its bracket: the last
/// index of the one starting at `start`, clamped to the score and cut short
/// before a measure where another ending starts (`<ending>`s can't nest).
fn ending_span_end(score: &Score, start: usize, measure_count: usize, ending: &Ending) -> usize {
    let nominal = (start + ending.measures.max(1) as usize).min(measure_count);
    (start + 1..nominal)
        .find(|&index| shared_measure(score, index).is_some_and(|m| m.barline.ending.is_some()))
        .map_or(nominal - 1, |next| next - 1)
}

/// Repeats/endings/navigation are measure-level in our model but shared
/// across a whole system in real MEI notation - like MusicXML's plugin, the
/// first part with a measure at this index wins.
fn shared_measure(score: &Score, measure_index: usize) -> Option<&Measure> {
    score
        .parts
        .iter()
        .find_map(|p| p.measures.get(measure_index))
}

fn write_measure(
    writer: &mut Writer<Vec<u8>>,
    score: &Score,
    measure_index: usize,
    beats: &mut u8,
) -> io::Result<()> {
    let shared = shared_measure(score, measure_index);
    let number = shared
        .map(|m| m.number)
        .unwrap_or((measure_index + 1) as u32);
    let barline = shared.map(|m| m.barline.clone()).unwrap_or_default();
    let navigation = shared.map(|m| m.navigation.as_slice()).unwrap_or_default();
    if let Some(ts) = shared.and_then(|m| m.attributes.time_signature) {
        *beats = ts.beats;
    }

    let mut el = writer
        .create_element("measure")
        .with_attribute(("n", number.to_string().as_str()));
    if barline.repeat_start {
        el = el.with_attribute(("left", "rptstart"));
    }
    let right_value;
    if let Some(times) = barline.repeat_end {
        right_value = times.to_string();
        el = el.with_attribute(("right", "rptend"));
        if times != 2 {
            el = el.with_attribute(("repeat.count", right_value.as_str()));
        }
    } else if let Some(rend) = barline_type_to_rend(barline.kind) {
        el = el.with_attribute(("right", rend));
    }

    el.write_inner_content(|writer| {
        for (part_index, part) in score.parts.iter().enumerate() {
            if let Some(measure) = part.measures.get(measure_index) {
                write_staff(writer, part_index, measure, measure_index == 0)?;
            }
        }
        for &mark in navigation {
            write_navigation(writer, mark, *beats)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// A `<repeatMark>` control event on the first staff: segno/coda signs at
/// the first beat, jumps and "Fine" at the closing barline (MEI's
/// `tstamp = meter.count + 1`).
fn write_navigation(
    writer: &mut Writer<Vec<u8>>,
    navigation: Navigation,
    beats: u8,
) -> io::Result<()> {
    let (func, text) = navigation_to_repeat_mark(navigation);
    let tstamp = if navigation_at_measure_end(navigation) {
        beats as u32 + 1
    } else {
        1
    };
    let el = writer
        .create_element("repeatMark")
        .with_attribute(("func", func))
        .with_attribute(("staff", "1"))
        .with_attribute(("tstamp", tstamp.to_string().as_str()));
    if text.is_empty() {
        el.write_empty()?;
    } else {
        el.write_text_content(BytesText::new(text))?;
    }
    Ok(())
}

fn write_staff(
    writer: &mut Writer<Vec<u8>>,
    part_index: usize,
    measure: &Measure,
    is_first_measure: bool,
) -> io::Result<()> {
    writer
        .create_element("staff")
        .with_attribute(("n", (part_index + 1).to_string().as_str()))
        .write_inner_content(|writer| {
            for voice in &measure.voices {
                writer
                    .create_element("layer")
                    .with_attribute(("n", voice.number.to_string().as_str()))
                    .write_inner_content(|writer| {
                        // The initial clef/key/meter for measure 1 already live on <staffDef>;
                        // later changes are written inline at the start of the first layer.
                        if !is_first_measure && voice.number == 1 {
                            if let Some(clef) = measure.attributes.clef {
                                write_clef_change(writer, clef)?;
                            }
                            if let Some(key) = measure.attributes.key_signature {
                                write_key_change(writer, key)?;
                            }
                            if let Some(ts) = measure.attributes.time_signature {
                                write_meter_change(writer, ts)?;
                            }
                        }
                        write_events(writer, &voice.events)?;
                        Ok(())
                    })?;
            }
            Ok(())
        })?;
    Ok(())
}

fn write_clef_change(writer: &mut Writer<Vec<u8>>, clef: Clef) -> io::Result<()> {
    let (shape, line) = clef_to_shape_line(clef);
    writer
        .create_element("clef")
        .with_attribute(("shape", shape))
        .with_attribute(("line", line.to_string().as_str()))
        .write_empty()?;
    Ok(())
}

fn write_key_change(writer: &mut Writer<Vec<u8>>, key: KeySignature) -> io::Result<()> {
    writer
        .create_element("keySig")
        .with_attribute(("sig", key_sig_to_attr(key.fifths).as_str()))
        .with_attribute(("mode", mode_to_attr(key.mode)))
        .write_empty()?;
    Ok(())
}

fn write_meter_change(writer: &mut Writer<Vec<u8>>, ts: TimeSignature) -> io::Result<()> {
    writer
        .create_element("meterSig")
        .with_attribute(("count", ts.beats.to_string().as_str()))
        .with_attribute(("unit", ts.beat_type.to_string().as_str()))
        .write_empty()?;
    Ok(())
}

fn write_events(writer: &mut Writer<Vec<u8>>, events: &[Event]) -> io::Result<()> {
    let mut i = 0;
    while i < events.len() {
        match events[i].duration().tuplet {
            Some(tuplet) => {
                let run_len = events[i..]
                    .iter()
                    .take_while(|e| e.duration().tuplet == Some(tuplet))
                    .count()
                    .max(1);
                writer
                    .create_element("tuplet")
                    .with_attribute(("num", tuplet.actual.to_string().as_str()))
                    .with_attribute(("numbase", tuplet.normal.to_string().as_str()))
                    .write_inner_content(|writer| {
                        for event in &events[i..i + run_len] {
                            write_event(writer, event)?;
                        }
                        Ok(())
                    })?;
                i += run_len;
            }
            None => {
                write_event(writer, &events[i])?;
                i += 1;
            }
        }
    }
    Ok(())
}

fn write_event(writer: &mut Writer<Vec<u8>>, event: &Event) -> io::Result<()> {
    match event {
        Event::Note(note) => {
            write_marking_siblings(writer, note)?;
            write_note(writer, note, true)
        }
        Event::Rest(rest) => {
            if let Some(symbol) = &rest.chord_symbol {
                write_harm(writer, symbol)?;
            }
            write_rest(writer, rest)
        }
        Event::Chord(notes) => {
            if let Some(first) = notes.first() {
                write_marking_siblings(writer, first)?;
            }
            let (dur, dots) = notes
                .first()
                .map(|n| (note_value_to_dur(n.duration.base), n.duration.dots))
                .unwrap_or((4, 0));
            writer
                .create_element("chord")
                .with_attribute(("dur", dur.to_string().as_str()))
                .with_attribute(("dots", dots.to_string().as_str()))
                .write_inner_content(|writer| {
                    for note in notes {
                        write_note(writer, note, false)?;
                    }
                    Ok(())
                })?;
            Ok(())
        }
    }
}

/// A chord symbol/dynamic/wedge marking is written as its own element
/// immediately before the note/chord it applies to (matching real MEI's
/// control-event style), rather than as an attribute on the note itself.
fn write_marking_siblings(writer: &mut Writer<Vec<u8>>, note: &Note) -> io::Result<()> {
    if let Some(symbol) = &note.chord_symbol {
        write_harm(writer, symbol)?;
    }
    if let Some(dynamic) = &note.dynamic {
        writer
            .create_element("dynam")
            .write_text_content(BytesText::new(&dynamic_to_text(dynamic)))?;
    }
    if let Some(wedge) = note.wedge {
        writer
            .create_element("hairpin")
            .with_attribute(("form", wedge_to_form(wedge)))
            .write_empty()?;
    }
    Ok(())
}

/// `<harm>` carries the chord symbol as text, in the shared
/// [`ChordSymbol`] text form.
fn write_harm(writer: &mut Writer<Vec<u8>>, symbol: &ChordSymbol) -> io::Result<()> {
    writer
        .create_element("harm")
        .write_text_content(BytesText::new(&symbol.to_string()))?;
    Ok(())
}

fn write_note(writer: &mut Writer<Vec<u8>>, note: &Note, with_duration: bool) -> io::Result<()> {
    let mut el = writer
        .create_element("note")
        .with_attribute(("pname", step_to_pname(note.pitch.step)))
        .with_attribute(("oct", note.pitch.octave.to_string().as_str()));
    let (dur_str, dots_str);
    if with_duration {
        dur_str = note_value_to_dur(note.duration.base).to_string();
        dots_str = note.duration.dots.to_string();
        el = el
            .with_attribute(("dur", dur_str.as_str()))
            .with_attribute(("dots", dots_str.as_str()));
    }
    if let Some(accid) = crate::convert::alter_to_accid_ges(note.pitch.alter) {
        el = el.with_attribute(("accid.ges", accid));
    }
    if let Some(tie) = tie_to_attr(note.tie) {
        el = el.with_attribute(("tie", tie));
    }
    if note.grace {
        el = el.with_attribute(("grace", "unacc"));
    }
    // Real MEI's att.fermataPresent shorthand for a `<fermata>` control event.
    if note.fermata {
        el = el.with_attribute(("fermata", "above"));
    }

    let has_markings = !note.articulations.is_empty() || !note.ornaments.is_empty();
    if has_markings {
        el.write_inner_content(|writer| {
            for articulation in &note.articulations {
                writer
                    .create_element("artic")
                    .with_attribute(("artic", articulation_to_code(articulation).as_str()))
                    .write_empty()?;
            }
            for ornament in &note.ornaments {
                write_ornament(writer, ornament)?;
            }
            Ok(())
        })?;
    } else {
        el.write_empty()?;
    }
    Ok(())
}

fn write_ornament(
    writer: &mut Writer<Vec<u8>>,
    ornament: &scores_core::Ornament,
) -> io::Result<()> {
    match ornament_to_element(ornament) {
        OrnamentElement::Trill => {
            writer.create_element("trill").write_empty()?;
        }
        OrnamentElement::Mordent { upper } => {
            writer
                .create_element("mordent")
                .with_attribute(("form", if upper { "upper" } else { "lower" }))
                .write_empty()?;
        }
        OrnamentElement::Turn { upper } => {
            writer
                .create_element("turn")
                .with_attribute(("form", if upper { "upper" } else { "lower" }))
                .write_empty()?;
        }
        OrnamentElement::Generic { kind, count } => {
            let count_str = count.map(|c| c.to_string());
            let mut el = writer
                .create_element("ornam")
                .with_attribute(("type", kind.as_str()));
            if let Some(count_str) = &count_str {
                el = el.with_attribute(("count", count_str.as_str()));
            }
            el.write_empty()?;
        }
    }
    Ok(())
}

fn write_rest(writer: &mut Writer<Vec<u8>>, rest: &Rest) -> io::Result<()> {
    let mut el = writer
        .create_element("rest")
        .with_attribute((
            "dur",
            note_value_to_dur(rest.duration.base).to_string().as_str(),
        ))
        .with_attribute(("dots", rest.duration.dots.to_string().as_str()));
    if rest.fermata {
        el = el.with_attribute(("fermata", "above"));
    }
    el.write_empty()?;
    Ok(())
}
