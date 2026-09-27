//! How notated performance marks are rendered into (and, where it can be
//! done unambiguously, recovered from) MIDI's timing and velocity.
//!
//! All of this only changes *sounding* length, onset or velocity: a part's
//! notated timeline (where each event starts) is never moved, so tracks stay
//! in sync with each other and with the conductor track.
//!
//! - Articulations: staccatissimo/staccato/spiccato/detached-legato sound for
//!   a fraction of the notated length ([`sounding_ticks`]); accent and strong
//!   accent (marcato) raise that one note's velocity by a fixed amount
//!   ([`accented_velocity`]), which [`accent_from_boost`] recognizes again on
//!   reading. Tenuto is full length, which is already the default.
//! - Fermata: the conductor track halves the tempo for the fermata event's
//!   span ([`FERMATA_STRETCH`]) and the part track carries a
//!   [`FERMATA_MARKER`] `Marker` at the event's start, so a reader can both
//!   restore the fermata and undo the tempo dip.
//! - Grace notes: see [`grace_length`].
//! - Ornaments: see [`realize_ornament`].

use scores_core::{Articulation, KeySignature, Ornament, Pitch, Step};

/// Ticks per quarter note used by the writer.
pub(crate) const PPQ: u16 = 480;

/// Text of the `Marker` meta event placed on an event carrying a fermata.
pub(crate) const FERMATA_MARKER: &str = "fermata";
/// A fermata event is played this many times slower (as a tempo dip, so it
/// doesn't shift the ticks of anything that follows).
pub(crate) const FERMATA_STRETCH: u32 = 2;

const ACCENT_BOOST: u8 = 20;
const STRONG_ACCENT_BOOST: u8 = 32;

/// Default length of one grace note (a 32nd note's worth of ticks).
const GRACE_TICKS: u32 = PPQ as u32 / 8;
/// Length of one ornament note (trill alternation, mordent/turn note).
const ORNAMENT_TICKS: u32 = PPQ as u32 / 8;

/// The sounding length of a note with `articulations` notated `ticks` long.
pub(crate) fn sounding_ticks(ticks: u32, articulations: &[&Articulation]) -> u32 {
    let (numerator, denominator) = articulations
        .iter()
        .map(|articulation| match articulation {
            Articulation::Staccatissimo => (1, 4),
            Articulation::Staccato | Articulation::Spiccato => (1, 2),
            Articulation::DetachedLegato => (3, 4),
            _ => (1, 1),
        })
        .min_by(|a, b| (a.0 * b.1).cmp(&(b.0 * a.1)))
        .unwrap_or((1, 1));
    (ticks * numerator / denominator).max(1)
}

/// `velocity` raised for an accent/strong accent (the larger one if both).
pub(crate) fn accented_velocity(velocity: u8, articulations: &[&Articulation]) -> u8 {
    let boost = articulations
        .iter()
        .map(|articulation| match articulation {
            Articulation::StrongAccent => STRONG_ACCENT_BOOST,
            Articulation::Accent => ACCENT_BOOST,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    velocity.saturating_add(boost).clamp(1, 127)
}

/// The accent a note's velocity rise over the surrounding dynamic level
/// indicates, if it's exactly one the writer produces.
pub(crate) fn accent_from_boost(boost: i16) -> Option<Articulation> {
    match boost {
        b if b == ACCENT_BOOST as i16 => Some(Articulation::Accent),
        b if b == STRONG_ACCENT_BOOST as i16 => Some(Articulation::StrongAccent),
        _ => None,
    }
}

/// Grace notes take no notated time, so `count` of them are played as a run
/// of short notes at the start of the following event, each this long:
/// normally [`GRACE_TICKS`], shrunk to fit in half of a following note (whose
/// onset is then delayed by the run, keeping its end) or in all of a
/// following rest. With no following event at all, the run is simply played
/// after the end of the part.
pub(crate) fn grace_length(count: u32, following: Option<(u32, bool)>) -> u32 {
    let budget = match following {
        Some((ticks, true)) => ticks / 2,
        Some((ticks, false)) => ticks,
        None => GRACE_TICKS * count,
    };
    GRACE_TICKS.min(budget / count.max(1)).max(1)
}

/// Realizes the first renderable ornament of a single untied note as
/// `(offset, length, key)` segments filling `sounding` ticks (or `None` to
/// play the note plainly). Neighbour notes are diatonic in `key_signature`
/// (the note's own accidentals aren't carried over to its neighbours):
///
/// - trill (also a shake, or a bare wavy line): alternates principal and
///   upper neighbour, starting on the principal;
/// - mordent (principal-lower-principal) and inverted mordent (with the
///   upper neighbour), at the start of the note;
/// - turn (upper-principal-lower-principal) and inverted turn at the start
///   of the note, delayed turns at its end;
/// - tremolo with `n` strokes: repeated notes of an eighth divided by
///   `2^(n-1)`.
///
/// Vertical/Haydn turns, the Schleifer and `Other` aren't rendered.
pub(crate) fn realize_ornament(
    pitch: Pitch,
    sounding: u32,
    ornaments: &[Ornament],
    key_signature: KeySignature,
) -> Option<Vec<(u32, u32, u8)>> {
    let principal = clamp_key(pitch.to_midi_number());
    let upper = neighbour(pitch, key_signature, true);
    let lower = neighbour(pitch, key_signature, false);

    ornaments.iter().find_map(|ornament| match ornament {
        Ornament::TrillMark | Ornament::Shake | Ornament::WavyLine => {
            alternate(sounding, ORNAMENT_TICKS, &[principal, upper])
        }
        Ornament::Tremolo(strokes) if *strokes > 0 => {
            let unit = (PPQ as u32 / 2) >> (strokes - 1).min(5);
            alternate(sounding, unit.max(1), &[principal])
        }
        Ornament::Mordent => figure(sounding, &[principal, lower], principal, false),
        Ornament::InvertedMordent => figure(sounding, &[principal, upper], principal, false),
        Ornament::Turn => figure(sounding, &[upper, principal, lower], principal, false),
        Ornament::InvertedTurn => figure(sounding, &[lower, principal, upper], principal, false),
        Ornament::DelayedTurn => figure(sounding, &[upper, principal, lower], principal, true),
        Ornament::DelayedInvertedTurn => {
            figure(sounding, &[lower, principal, upper], principal, true)
        }
        _ => None,
    })
}

/// Cycles through `keys` every `unit` ticks (the last one taking the
/// remainder); `None` if fewer than two notes would fit.
fn alternate(sounding: u32, unit: u32, keys: &[u8]) -> Option<Vec<(u32, u32, u8)>> {
    let count = sounding / unit;
    if count < 2 {
        return None;
    }
    Some(
        (0..count)
            .map(|index| {
                let offset = index * unit;
                let length = if index + 1 == count {
                    sounding - offset
                } else {
                    unit
                };
                (offset, length, keys[index as usize % keys.len()])
            })
            .collect(),
    )
}

/// Plays the short `notes` figure followed by the held `principal`, at the
/// start of the note (or at its end, after holding the principal, when
/// `delayed`). Each figure note is [`ORNAMENT_TICKS`] long, shrunk so the
/// figure takes at most half of the note.
fn figure(
    sounding: u32,
    notes: &[u8],
    principal: u8,
    delayed: bool,
) -> Option<Vec<(u32, u32, u8)>> {
    let figure_count = notes.len() as u32 + 1;
    let unit = ORNAMENT_TICKS.min(sounding / (2 * figure_count));
    if unit == 0 {
        return None;
    }
    let mut segments = Vec::new();
    let mut offset = 0;
    if delayed {
        let hold = sounding - unit * figure_count;
        segments.push((0, hold, principal));
        offset = hold;
    }
    for &key in notes {
        segments.push((offset, unit, key));
        offset += unit;
    }
    segments.push((offset, sounding - offset, principal));
    Some(segments)
}

/// The next diatonic step above/below `pitch`, with the alteration
/// `key_signature` gives that step.
fn neighbour(pitch: Pitch, key_signature: KeySignature, up: bool) -> u8 {
    const STEPS: [Step; 7] = [
        Step::C,
        Step::D,
        Step::E,
        Step::F,
        Step::G,
        Step::A,
        Step::B,
    ];
    let index = STEPS.iter().position(|&s| s == pitch.step).unwrap_or(0) as i32;
    let target = index + if up { 1 } else { -1 };
    let step = STEPS[target.rem_euclid(7) as usize];
    let octave = pitch.octave as i32 + target.div_euclid(7);
    let neighbour = Pitch::new(step, key_alter(step, key_signature), octave as i8);
    clamp_key(neighbour.to_midi_number())
}

/// The alteration `key_signature` applies to `step`.
fn key_alter(step: Step, key_signature: KeySignature) -> i8 {
    const SHARPS: [Step; 7] = [
        Step::F,
        Step::C,
        Step::G,
        Step::D,
        Step::A,
        Step::E,
        Step::B,
    ];
    let fifths = key_signature.fifths.clamp(-7, 7);
    if fifths > 0 && SHARPS[..fifths as usize].contains(&step) {
        1
    } else if fifths < 0 && SHARPS[SHARPS.len() - fifths.unsigned_abs() as usize..].contains(&step)
    {
        -1
    } else {
        0
    }
}

pub(crate) fn clamp_key(midi_number: i32) -> u8 {
    midi_number.clamp(0, 127) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use scores_core::Mode;

    fn key(fifths: i8) -> KeySignature {
        KeySignature {
            fifths,
            mode: Mode::Major,
        }
    }

    #[test]
    fn neighbours_follow_the_key_signature() {
        let e4 = Pitch::new(Step::E, 0, 4);
        // In D major (F#, C#) the note above E is F#; in C major it's F.
        assert_eq!(neighbour(e4, key(2), true), 66);
        assert_eq!(neighbour(e4, key(0), true), 65);
        // In F major (Bb) the note below C5 is Bb4.
        assert_eq!(neighbour(Pitch::new(Step::C, 0, 5), key(-1), false), 70);
    }

    #[test]
    fn trill_alternates_and_fills_the_note() {
        let segments = realize_ornament(
            Pitch::new(Step::C, 0, 4),
            480,
            &[Ornament::TrillMark],
            key(0),
        )
        .unwrap();
        assert_eq!(segments.len(), 8);
        assert_eq!(segments[0], (0, 60, 60));
        assert_eq!(segments[1], (60, 60, 62));
        let total: u32 = segments.iter().map(|s| s.1).sum();
        assert_eq!(total, 480);
    }

    #[test]
    fn mordent_then_held_principal() {
        let segments =
            realize_ornament(Pitch::new(Step::C, 0, 4), 480, &[Ornament::Mordent], key(0)).unwrap();
        assert_eq!(segments, vec![(0, 60, 60), (60, 60, 59), (120, 360, 60)]);
    }

    #[test]
    fn staccato_halves_and_accent_boosts() {
        assert_eq!(sounding_ticks(480, &[&Articulation::Staccato]), 240);
        assert_eq!(
            sounding_ticks(
                480,
                &[&Articulation::Staccato, &Articulation::Staccatissimo]
            ),
            120
        );
        assert_eq!(sounding_ticks(480, &[&Articulation::Tenuto]), 480);
        assert_eq!(accented_velocity(80, &[&Articulation::Accent]), 100);
        assert_eq!(accented_velocity(120, &[&Articulation::StrongAccent]), 127);
        assert_eq!(accent_from_boost(20), Some(Articulation::Accent));
    }
}
