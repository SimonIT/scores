use crate::convert::{
    FERMATA_TOKEN, dynamic_from_token, navigation_from_text, navigation_precedes_measure,
};
use chordpro::{Chunk, Section, Song};
use scores_core::{
    ChordKind, ChordSymbol, Duration, Dynamic, Event, Measure, MeasureAttributes, Navigation, Note,
    NoteValue, Part, Pitch, Rest, Result, Score, ScoresError, Tie,
};
use std::str::FromStr;

/// What the reader substitutes for every `[chord]` before handing the text
/// to the `chordpro` crate - see [`extract_chords`].
const PLACEHOLDER_CHORD: &str = "[C]";

/// ChordPro has no notated rhythm at all - only chord symbols positioned in
/// lines of lyrics - so lyrics are dropped (our model has no lyric field)
/// and every chord becomes a fixed quarter-note [`Event`], one measure per
/// source line that contains at least one chord (chord-less lines are
/// skipped entirely).
///
/// Each event carries the chord as a [`ChordSymbol`] (parsed from the
/// chord's exact source text, see [`extract_chords`]) *and* is voiced as
/// notes via [`voice_chord`], so formats without chord symbols still get
/// something playable. `[N.C.]` becomes a rest carrying the no-chord
/// symbol. Bracketed text that isn't a chord symbol at all (e.g. a ChordPro
/// `[*annotation]`) is ignored.
///
/// Dynamics and fermatas ride along via [`crate::convert`]'s bolt-on
/// `<token>` convention: a bracket-free text chunk immediately before a
/// chord marks that chord (any lyric text in between clears it, so a
/// marker only ever applies to the chord right after it).
///
/// A `{comment: ...}` whose whole text is a navigation mark ("D.S. al
/// Coda", "Fine", "Segno", ...) is added to [`Measure::navigation`]:
/// segno/coda on the next measure, the jump/stop instructions on the
/// previous one (the placement [`crate::write`] uses). Other comments are ignored.
pub fn read(data: &[u8]) -> Result<Score> {
    let text = std::str::from_utf8(data)
        .map_err(|e| ScoresError::Parse(format!("input is not valid utf-8: {e}")))?;
    let (text, chord_texts) = extract_chords(text);
    let song =
        Song::from_str(&text).map_err(|e| ScoresError::Parse(format!("invalid ChordPro: {e}")))?;
    let mut chord_texts = chord_texts.into_iter();

    let mut measures: Vec<Measure> = Vec::new();
    let mut pending_navigation: Vec<Navigation> = Vec::new();
    for section in &song.song {
        let paragraph = match section {
            Section::Verse(paragraph) | Section::Chorus(paragraph) => paragraph,
            Section::Comment(line) => {
                let text: String = line
                    .0
                    .iter()
                    .filter_map(|chunk| match chunk {
                        Chunk::Lyrics(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                if let Some(navigation) = navigation_from_text(&text) {
                    match measures.last_mut() {
                        Some(last) if !navigation_precedes_measure(navigation) => {
                            last.navigation.push(navigation);
                        }
                        _ => pending_navigation.push(navigation),
                    }
                }
                continue;
            }
            _ => continue,
        };
        for line in &paragraph.0 {
            let mut events = Vec::new();
            let mut pending_dynamic: Option<Dynamic> = None;
            let mut pending_fermata = false;

            for chunk in &line.0 {
                match chunk {
                    Chunk::Lyrics(text) => match parse_marker(text) {
                        Some(Marker::Dynamic(d)) => pending_dynamic = Some(d),
                        Some(Marker::Fermata) => pending_fermata = true,
                        None => {
                            pending_dynamic = None;
                            pending_fermata = false;
                        }
                    },
                    Chunk::Chord(_) => {
                        // The crate only ever saw our placeholder here; the
                        // real text comes from `extract_chords`, in the same order.
                        let Some(symbol) = chord_texts
                            .next()
                            .and_then(|text| ChordSymbol::parse(&text))
                        else {
                            continue;
                        };
                        events.push(chord_event(symbol, pending_dynamic.take(), pending_fermata));
                        pending_fermata = false;
                    }
                    _ => {}
                }
            }

            if events.is_empty() {
                continue;
            }
            let mut measure = Measure::single_voice(
                (measures.len() + 1) as u32,
                MeasureAttributes::default(),
                events,
            );
            measure.navigation = std::mem::take(&mut pending_navigation);
            measures.push(measure);
        }
    }

    if measures.is_empty() {
        return Err(ScoresError::Parse(
            "ChordPro input contains no chords".into(),
        ));
    }
    // A trailing segno/coda with no measure after it still belongs to the song.
    if let Some(last) = measures.last_mut() {
        last.navigation.extend(pending_navigation);
    }

    let title = (!song.title.is_empty()).then_some(song.title);
    // ChordPro has no dedicated composer field; `artist` is the closest equivalent.
    let composer = (!song.artist.is_empty()).then_some(song.artist);

    Ok(Score {
        title,
        composer,
        parts: vec![Part {
            id: "P1".to_string(),
            name: None,
            measures,
        }],
    })
}

/// The `chordpro` crate's chord grammar is far narrower than real-world
/// ChordPro: it loses spelling (`Eb` becomes `D#`) and qualities (`Cmaj7`
/// parses as `C7`), and rejects the *whole song* on anything it doesn't
/// know (`F#m7b5`, `C13`, `N.C.`). So every `[...]` in a lyric line is
/// swapped for [`PLACEHOLDER_CHORD`] before parsing, and its original text
/// returned alongside, in order - one entry per `Chunk::Chord` the crate
/// will produce in verse/chorus lines. `{...}` directives (whose own chords
/// end up in comment sections, which the reader doesn't voice) and `#`
/// comment lines are left untouched so the two sequences stay aligned.
fn extract_chords(text: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut chords = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with('#') {
            out.push_str(line);
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find(['[', '{']) {
            let close = if rest[start..].starts_with('[') {
                ']'
            } else {
                '}'
            };
            // An unclosed bracket is left as-is for the crate to reject.
            let Some(len) = rest[start..].find(close) else {
                break;
            };
            let end = start + len + 1;
            out.push_str(&rest[..start]);
            if close == ']' {
                chords.push(rest[start + 1..end - 1].to_string());
                out.push_str(PLACEHOLDER_CHORD);
            } else {
                out.push_str(&rest[start..end]);
            }
            rest = &rest[end..];
        }
        out.push_str(rest);
    }
    (out, chords)
}

enum Marker {
    Dynamic(Dynamic),
    Fermata,
}

/// Recognizes a lyric chunk that is *exactly* (after trimming) a `<token>`
/// marker - anything else (including a marker with surrounding lyric text)
/// is not a marker.
fn parse_marker(text: &str) -> Option<Marker> {
    let token = text.trim().strip_prefix('<')?.strip_suffix('>')?;
    if token.eq_ignore_ascii_case(FERMATA_TOKEN) {
        Some(Marker::Fermata)
    } else {
        dynamic_from_token(token).map(Marker::Dynamic)
    }
}

fn chord_event(symbol: ChordSymbol, dynamic: Option<Dynamic>, fermata: bool) -> Event {
    let pitches = voice_chord(&symbol);
    let mut event = match pitches.len() {
        // Only "N.C." has no chord tones.
        0 => {
            let mut rest = Rest::new(Duration::new(NoteValue::Quarter));
            rest.fermata = fermata;
            Event::Rest(rest)
        }
        _ => {
            let mut notes: Vec<Note> = pitches
                .into_iter()
                .map(|pitch| {
                    let mut note = Note::new(pitch, Duration::new(NoteValue::Quarter), Tie::None);
                    note.dynamic = dynamic.clone();
                    note.fermata = fermata;
                    note
                })
                .collect();
            if notes.len() == 1 {
                Event::Note(notes.remove(0))
            } else {
                Event::Chord(notes)
            }
        }
    };
    event.set_chord_symbol(Some(symbol));
    event
}

/// Voices a chord symbol in close position from the root at or above middle
/// C (so the root is always the lowest chord tone, matching the writer's
/// "lowest note is the bass" recognition on the way back), with a slash
/// chord's bass an octave below. Pitches are spelled with flats if the root
/// is.
fn voice_chord(symbol: &ChordSymbol) -> Vec<Pitch> {
    let intervals = match &symbol.kind {
        ChordKind::Other(suffix) => base_kind(suffix).intervals(),
        kind => kind.intervals(),
    };
    let prefer_flats = symbol.root.alter < 0;
    let root_midi = 60 + i32::from(symbol.root.pitch_class());

    let mut pitches: Vec<Pitch> = intervals
        .iter()
        .map(|&offset| Pitch::from_midi_number(root_midi + i32::from(offset), prefer_flats))
        .collect();

    if let Some(bass) = symbol.bass
        && bass.pitch_class() != symbol.root.pitch_class()
        && !pitches.is_empty()
    {
        pitches.insert(
            0,
            Pitch::from_midi_number(48 + i32::from(bass.pitch_class()), bass.alter < 0),
        );
    }

    pitches
}

/// Suffixes [`ChordKind`] has no variant for (`7b9`, `madd9`, `maj7#11`)
/// are voiced as their longest recognizable leading quality (`7`, `m`,
/// `maj7`), cutting only where an alteration or added/suspended tone
/// begins - never mid-word, so `madd9` is `m` + `add9`, not `ma` + `dd9`.
/// Falls back to a major triad.
fn base_kind(suffix: &str) -> ChordKind {
    const MODIFIERS: [&str; 5] = ["add", "sus", "alt", "omit", "no"];
    for (end, _) in suffix.char_indices().rev() {
        let (head, rest) = suffix.split_at(end);
        let mid_word = rest.starts_with(|c: char| c.is_alphabetic() && c != 'b')
            && !MODIFIERS.iter().any(|m| rest.starts_with(m));
        if mid_word {
            continue;
        }
        let kind = ChordKind::from_suffix(head);
        if !matches!(kind, ChordKind::Other(_)) {
            return kind;
        }
    }
    ChordKind::Major
}
