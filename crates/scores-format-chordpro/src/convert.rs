use scores_core::{Dynamic, Navigation};

/// Our own bolt-on convention for carrying a dynamic marking through
/// ChordPro (which has no native syntax for one): a bracket-free `<token>`
/// text chunk immediately before the chord it modifies. `<ferm>` similarly
/// marks a fermata. Neither is standard ChordPro; both are inert to the
/// `chordpro` crate's own parser (unrecognized text just becomes a lyric).
pub(crate) const FERMATA_TOKEN: &str = "ferm";

pub(crate) fn dynamic_to_token(dynamic: &Dynamic) -> Option<&'static str> {
    Some(match dynamic {
        Dynamic::P => "p",
        Dynamic::Pp => "pp",
        Dynamic::Ppp => "ppp",
        Dynamic::Pppp => "pppp",
        Dynamic::Ppppp => "ppppp",
        Dynamic::Pppppp => "pppppp",
        Dynamic::F => "f",
        Dynamic::Ff => "ff",
        Dynamic::Fff => "fff",
        Dynamic::Ffff => "ffff",
        Dynamic::Fffff => "fffff",
        Dynamic::Ffffff => "ffffff",
        Dynamic::Mp => "mp",
        Dynamic::Mf => "mf",
        Dynamic::Sf => "sf",
        Dynamic::Sfp => "sfp",
        Dynamic::Sfpp => "sfpp",
        Dynamic::Fp => "fp",
        Dynamic::Rf => "rf",
        Dynamic::Rfz => "rfz",
        Dynamic::Sfz => "sfz",
        Dynamic::Sffz => "sffz",
        Dynamic::Fz => "fz",
        Dynamic::N => "n",
        Dynamic::Pf => "pf",
        Dynamic::Sfzp => "sfzp",
        // Arbitrary "other" text isn't part of this bolt-on convention.
        Dynamic::Other(_) => return None,
    })
}

pub(crate) fn dynamic_from_token(token: &str) -> Option<Dynamic> {
    Some(match token {
        "p" => Dynamic::P,
        "pp" => Dynamic::Pp,
        "ppp" => Dynamic::Ppp,
        "pppp" => Dynamic::Pppp,
        "ppppp" => Dynamic::Ppppp,
        "pppppp" => Dynamic::Pppppp,
        "f" => Dynamic::F,
        "ff" => Dynamic::Ff,
        "fff" => Dynamic::Fff,
        "ffff" => Dynamic::Ffff,
        "fffff" => Dynamic::Fffff,
        "ffffff" => Dynamic::Ffffff,
        "mp" => Dynamic::Mp,
        "mf" => Dynamic::Mf,
        "sf" => Dynamic::Sf,
        "sfp" => Dynamic::Sfp,
        "sfpp" => Dynamic::Sfpp,
        "fp" => Dynamic::Fp,
        "rf" => Dynamic::Rf,
        "rfz" => Dynamic::Rfz,
        "sfz" => Dynamic::Sfz,
        "sffz" => Dynamic::Sffz,
        "fz" => Dynamic::Fz,
        "n" => Dynamic::N,
        "pf" => Dynamic::Pf,
        "sfzp" => Dynamic::Sfzp,
        _ => return None,
    })
}

/// The text written in a standard `{comment: ...}` directive for a
/// navigation mark - the same wording a hand-written lead sheet would use.
pub(crate) fn navigation_to_text(navigation: Navigation) -> &'static str {
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

/// Recognizes a comment whose *whole* text is a navigation mark (case- and
/// whitespace-insensitive, also accepting the spelled-out "Da Capo"/"Dal
/// Segno"), so ordinary comments like "Play twice" are never misread.
pub(crate) fn navigation_from_text(text: &str) -> Option<Navigation> {
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .replace("da capo", "d.c.")
        .replace("dal segno", "d.s.");
    Some(match normalized.as_str() {
        "segno" => Navigation::Segno,
        "coda" => Navigation::Coda,
        "d.c." => Navigation::DaCapo,
        "d.c. al fine" => Navigation::DaCapoAlFine,
        "d.c. al coda" => Navigation::DaCapoAlCoda,
        "d.s." => Navigation::DalSegno,
        "d.s. al fine" => Navigation::DalSegnoAlFine,
        "d.s. al coda" => Navigation::DalSegnoAlCoda,
        "fine" => Navigation::Fine,
        "to coda" => Navigation::ToCoda,
        _ => return None,
    })
}

/// Whether a navigation mark belongs *before* its measure (a target you
/// jump to: segno, coda) rather than after it (an instruction you act on
/// once you reach the end of the measure: D.C., D.S., Fine, To Coda).
pub(crate) fn navigation_precedes_measure(navigation: Navigation) -> bool {
    matches!(navigation, Navigation::Segno | Navigation::Coda)
}
