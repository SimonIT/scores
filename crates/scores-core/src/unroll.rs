//! Realizes a part's repeat structure into the linear order its measures are
//! actually played in, for performance formats (MIDI, RTTTL) that are a
//! single timeline with no way to say "go back to bar 1".
//!
//! Interpretation (flat, non-nested - the same subset every notation plugin
//! models):
//!
//! - A `repeat_end` of `n` jumps back to the most recent `repeat_start` (or
//!   the start of the piece / the measure after the previous repeated
//!   section) until the section has been played `n` times.
//! - A measure inside a 1st/2nd/... ending is only played on the pass(es)
//!   listed in its [`Ending::numbers`](crate::Ending::numbers).
//! - Segno/Coda marks take effect at the start of their measure; D.C., D.S.,
//!   Fine and To Coda at its end (after any repeat of that measure has been
//!   taken). Only the first D.C./D.S. jump is followed. After it, repeats are
//!   not taken again and only the last ending is played (the common "senza
//!   replica" convention); "al Fine" stops at the end of the Fine measure,
//!   "al Coda" jumps from the To Coda measure to the Coda measure, and a plain
//!   D.C./D.S. plays on to the end. A D.S. with no Segno jumps to the start;
//!   a missing Fine/Coda target means playing on to the end.

use crate::model::{Measure, Navigation};

/// Where a followed D.C./D.S. jump ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum JumpEnd {
    End,
    Fine,
    Coda,
}

/// Returns indices into `measures` in playback order.
///
/// When a score has several parts, compute the order from one of them (they
/// share their repeat structure) and apply it to all, so they stay aligned.
pub fn playback_order(measures: &[Measure]) -> Vec<usize> {
    let endings = ending_passes(measures);
    let final_ending = endings
        .iter()
        .flatten()
        .flat_map(|n| n.iter())
        .copied()
        .max();
    let segno = position_of(measures, Navigation::Segno);
    let coda = position_of(measures, Navigation::Coda);
    // Every construct above only ever moves backwards a bounded number of
    // times, but a hard cap keeps malformed input from looping forever.
    let limit = measures.len().saturating_mul(64).max(64);

    let mut order = Vec::with_capacity(measures.len());
    let mut i = 0;
    let mut section_start = 0;
    let mut pass: u32 = 1;
    let mut jump: Option<JumpEnd> = None;
    let mut last_played: Option<usize> = None;
    let mut returning = false;

    while i < measures.len() && order.len() < limit {
        let measure = &measures[i];
        if !returning && jump.is_none() {
            let left_ending_group = endings[i].is_none()
                && last_played.is_some_and(|last| last < i && endings[last].is_some());
            if measure.barline.repeat_start || left_ending_group {
                section_start = i;
                pass = 1;
            }
        }
        returning = false;

        if let Some(numbers) = &endings[i] {
            let wanted = if jump.is_some() {
                final_ending.map(u32::from)
            } else {
                Some(pass)
            };
            if !numbers.iter().any(|&n| Some(u32::from(n)) == wanted) {
                i += 1;
                continue;
            }
        }

        order.push(i);
        last_played = Some(i);

        let marks = &measure.navigation;
        match jump {
            Some(JumpEnd::Fine) if marks.contains(&Navigation::Fine) => break,
            Some(JumpEnd::Coda) if marks.contains(&Navigation::ToCoda) => {
                if let Some(target) = coda.filter(|&target| target > i) {
                    i = target;
                    continue;
                }
            }
            _ => {}
        }

        if jump.is_none() {
            if let Some(times) = measure.barline.repeat_end {
                if pass < times {
                    pass += 1;
                    i = section_start;
                    returning = true;
                    continue;
                }
                section_start = i + 1;
                // Inside an ending the pass count still selects which
                // following ending measures play; it's reset once the
                // ending group is left instead.
                if endings[i].is_none() {
                    pass = 1;
                }
            }

            let target = marks.iter().find_map(|mark| match mark {
                Navigation::DaCapo => Some((0, JumpEnd::End)),
                Navigation::DaCapoAlFine => Some((0, JumpEnd::Fine)),
                Navigation::DaCapoAlCoda => Some((0, JumpEnd::Coda)),
                Navigation::DalSegno => Some((segno.unwrap_or(0), JumpEnd::End)),
                Navigation::DalSegnoAlFine => Some((segno.unwrap_or(0), JumpEnd::Fine)),
                Navigation::DalSegnoAlCoda => Some((segno.unwrap_or(0), JumpEnd::Coda)),
                _ => None,
            });
            if let Some((target, end)) = target {
                jump = Some(end);
                i = target;
                continue;
            }
        }

        i += 1;
    }

    order
}

fn position_of(measures: &[Measure], navigation: Navigation) -> Option<usize> {
    measures
        .iter()
        .position(|m| m.navigation.contains(&navigation))
}

/// For each measure, the pass numbers of the ending bracket it lies in (if
/// any). Brackets are clamped to the part's length, and a bracket starting
/// inside an earlier one cuts that earlier one short.
fn ending_passes(measures: &[Measure]) -> Vec<Option<&[u8]>> {
    let mut result = vec![None; measures.len()];
    for (index, measure) in measures.iter().enumerate() {
        if let Some(ending) = &measure.barline.ending {
            let last = (index + ending.measures.max(1) as usize).min(measures.len());
            for slot in &mut result[index..last] {
                *slot = Some(ending.numbers.as_slice());
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Barline, Ending, MeasureAttributes};

    fn measures(count: usize) -> Vec<Measure> {
        (0..count)
            .map(|i| Measure::single_voice(i as u32 + 1, MeasureAttributes::default(), vec![]))
            .collect()
    }

    fn ending(numbers: &[u8], measures: u32) -> Option<Ending> {
        Some(Ending {
            numbers: numbers.to_vec(),
            measures,
            closed: true,
        })
    }

    #[test]
    fn plain_score_plays_straight_through() {
        assert_eq!(playback_order(&measures(3)), vec![0, 1, 2]);
    }

    #[test]
    fn simple_repeat_without_start_goes_back_to_the_beginning() {
        let mut m = measures(3);
        m[1].barline.repeat_end = Some(2);
        assert_eq!(playback_order(&m), vec![0, 1, 0, 1, 2]);
    }

    #[test]
    fn repeat_count_and_start_are_honoured() {
        let mut m = measures(4);
        m[1].barline.repeat_start = true;
        m[2].barline.repeat_end = Some(3);
        assert_eq!(playback_order(&m), vec![0, 1, 2, 1, 2, 1, 2, 3]);
    }

    #[test]
    fn first_and_second_endings() {
        // |: 0 | 1 |[1 2 | 3 :|[2 4 | 5 |
        let mut m = measures(6);
        m[0].barline.repeat_start = true;
        m[2].barline.ending = ending(&[1], 2);
        m[3].barline = Barline {
            repeat_end: Some(2),
            ..Default::default()
        };
        m[4].barline.ending = ending(&[2], 1);
        assert_eq!(playback_order(&m), vec![0, 1, 2, 3, 0, 1, 4, 5]);
    }

    #[test]
    fn a_later_repeat_section_starts_fresh_after_an_ending_group() {
        let mut m = measures(5);
        m[1].barline.ending = ending(&[1], 1);
        m[1].barline.repeat_end = Some(2);
        m[2].barline.ending = ending(&[2], 1);
        m[4].barline.repeat_end = Some(2);
        assert_eq!(playback_order(&m), vec![0, 1, 0, 2, 3, 4, 3, 4]);
    }

    #[test]
    fn da_capo_al_fine_skips_repeats_on_the_second_time() {
        let mut m = measures(4);
        m[0].barline.repeat_end = Some(2);
        m[1].navigation = vec![Navigation::Fine];
        m[3].navigation = vec![Navigation::DaCapoAlFine];
        assert_eq!(playback_order(&m), vec![0, 0, 1, 2, 3, 0, 1]);
    }

    #[test]
    fn dal_segno_al_coda() {
        // 0 | Segno 1 | ToCoda 2 | 3 D.S. al Coda | Coda 4
        let mut m = measures(5);
        m[1].navigation = vec![Navigation::Segno];
        m[2].navigation = vec![Navigation::ToCoda];
        m[3].navigation = vec![Navigation::DalSegnoAlCoda];
        m[4].navigation = vec![Navigation::Coda];
        assert_eq!(playback_order(&m), vec![0, 1, 2, 3, 1, 2, 4]);
    }

    #[test]
    fn after_da_capo_only_the_last_ending_is_played() {
        let mut m = measures(4);
        m[1].barline.ending = ending(&[1], 1);
        m[1].barline.repeat_end = Some(2);
        m[2].barline.ending = ending(&[2], 1);
        m[3].navigation = vec![Navigation::DaCapo];
        assert_eq!(playback_order(&m), vec![0, 1, 0, 2, 3, 0, 2, 3]);
    }
}
