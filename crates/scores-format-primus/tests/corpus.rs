//! Round-trips real `.pri` files through the shared model: each file is
//! read, written back and read again, and the two readings compared
//! measure by measure. The files aren't part of the repo, so the test is
//! ignored by default: point `PRI_SAMPLES_DIR` at a directory of `.pri`
//! files (searched recursively) and run
//! `cargo test -p scores-format-primus --test corpus -- --ignored --nocapture`.

use std::path::{Path, PathBuf};

use scores_core::{Event, FormatReader, FormatWriter, Voice};
use scores_format_primus::PrimusFormat;

fn pri_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "Autosave") {
                continue;
            }
            pri_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pri"))
        {
            out.push(path);
        }
    }
}

/// A measure's voices without the trailing rests the writer adds to fill
/// a voice up to its measure's length (PriMus needs every voice to fill
/// it), and without voices left empty.
fn normalized(voices: &[Voice]) -> Vec<Vec<Event>> {
    voices
        .iter()
        .map(|v| {
            let mut events = v.events.clone();
            while matches!(events.last(), Some(Event::Rest(r)) if !r.fermata && r.chord_symbol.is_none()) {
                events.pop();
            }
            events
        })
        .filter(|events| !events.is_empty())
        .collect()
}

#[test]
#[ignore]
fn round_trips_real_files() {
    let dir =
        std::env::var("PRI_SAMPLES_DIR").expect("set PRI_SAMPLES_DIR to a directory of .pri files");
    let mut files = Vec::new();
    pri_files(Path::new(&dir), &mut files);
    assert!(!files.is_empty(), "no .pri files under {dir}");
    let (mut read, mut measures, mut differing) = (0, 0, 0);
    let mut failures = Vec::new();
    for path in &files {
        let Ok(score) = PrimusFormat.read(&std::fs::read(path).unwrap()) else {
            println!("skipped (not readable): {}", path.display());
            continue;
        };
        read += 1;
        let back = match PrimusFormat
            .write(&score)
            .and_then(|b| PrimusFormat.read(&b))
        {
            Ok(back) => back,
            Err(e) => {
                failures.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        if back.parts.len() != score.parts.len() {
            failures.push(format!(
                "{}: {} parts, {} after",
                path.display(),
                score.parts.len(),
                back.parts.len()
            ));
            continue;
        }
        let mut first = None;
        for (p, (a, b)) in score.parts.iter().zip(&back.parts).enumerate() {
            for (m, (ma, mb)) in a.measures.iter().zip(&b.measures).enumerate() {
                measures += 1;
                if normalized(&ma.voices) != normalized(&mb.voices) {
                    differing += 1;
                    if first.is_none() && std::env::var_os("PRI_DIFF").is_some() {
                        println!("  before: {:?}\n  after:  {:?}", ma.voices, mb.voices);
                    }
                    first.get_or_insert((p, m));
                }
            }
        }
        if let Some((p, m)) = first {
            println!(
                "{}: first difference in part {p}, measure {}",
                path.display(),
                m + 1
            );
        }
    }
    println!("{read} files, {measures} measures, {differing} differing");
    for f in &failures {
        println!("{f}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
}
