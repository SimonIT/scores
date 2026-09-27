//! MIDI has no notated-dynamics concept, only per-note velocity (0-127), so
//! [`Dynamic`] markings are approximated by bucketing/unbucketing velocity.
//! This is necessarily lossy in both directions - many different dynamic
//! markings collapse to the same velocity bucket, and a `Dynamic::Other`
//! (or any marking with no obvious loudness, like `N`) falls back to the
//! neutral default velocity.

use scores_core::Dynamic;

pub(crate) const DEFAULT_VELOCITY: u8 = 80;

pub(crate) fn velocity_to_dynamic(velocity: u8) -> Dynamic {
    match velocity {
        0..=15 => Dynamic::Ppp,
        16..=31 => Dynamic::Pp,
        32..=47 => Dynamic::P,
        48..=63 => Dynamic::Mp,
        64..=79 => Dynamic::Mf,
        80..=95 => Dynamic::F,
        96..=111 => Dynamic::Ff,
        _ => Dynamic::Fff,
    }
}

pub(crate) fn dynamic_to_velocity(dynamic: Option<&Dynamic>) -> u8 {
    match dynamic {
        None => DEFAULT_VELOCITY,
        Some(dynamic) => match dynamic {
            Dynamic::Pppppp | Dynamic::Ppppp | Dynamic::Pppp => 4,
            Dynamic::Ppp => 8,
            Dynamic::Pp => 24,
            Dynamic::P => 40,
            Dynamic::Mp => 56,
            Dynamic::Mf => 72,
            Dynamic::F => 88,
            Dynamic::Ff => 104,
            Dynamic::Fff | Dynamic::Ffff | Dynamic::Fffff | Dynamic::Ffffff => 120,
            Dynamic::Sf
            | Dynamic::Sfz
            | Dynamic::Sffz
            | Dynamic::Rf
            | Dynamic::Rfz
            | Dynamic::Fz => 112,
            Dynamic::Sfp | Dynamic::Sfpp | Dynamic::Fp | Dynamic::Sfzp => 96,
            Dynamic::Pf => 64,
            Dynamic::N => 20,
            Dynamic::Other(_) => DEFAULT_VELOCITY,
        },
    }
}
