# scores

A Rust workspace for converting between music score formats.

```
crates/
  scores-core/              shared data model (Score/Part/Measure/Note/ChordSymbol/...),
                             the Format/FormatReader/FormatWriter/FormatRegistry traits,
                             and repeat unrolling (`playback_order`)
  scores-format-abc/        ABC notation, on top of `abc-parser`
  scores-format-musicxml/   MusicXML, on top of `musicxml`
  scores-format-midi/       Standard MIDI files, on top of `midly`
  scores-format-mei/        MEI (Music Encoding Initiative), hand-rolled on `quick-xml`
                             (no suitable MEI crate exists on crates.io)
  scores-format-guitarpro/  Guitar Pro 3/4/5 (binary tab format), on top of `guitarpro`
  scores-format-chordpro/   ChordPro (chords + lyrics lead sheets), on top of `chordpro`
  scores-format-rtttl/      RTTTL (ringtone melodies), on top of `rtttl` (read-only crate;
                             writing is hand-rolled)
  scores-format-musescore/  MuseScore 4 (.mscz/.mscx), on top of `guitarpro`'s converter
                             plus a hand-rolled `quick-xml` pass for notation details
  scores-format-primus/     PriMus (.pri), on top of `primus-scores` (a path dependency on a
                             sibling checkout, `../primus-scores`); writes into an embedded
                             PriMus Free template
  scores-cli/               `scores` binary: convert between formats on the command line
  scores-web/                wasm-bindgen bindings + a browser demo page
```

## Architecture

Format support is a **static plugin**: each `scores-format-*` crate implements
`scores_core::Format` for its own unit struct, converting to and from the shared
`Score` model. `scores-cli` and `scores-web` each depend on whichever format
crates they want and register them into a `FormatRegistry` at startup. There is
no runtime/dynamic plugin loading (no `dlopen`) - that keeps every format usable
identically from a native binary and from WebAssembly.

Every plugin documents its own scope/limitations in its crate-level docs
(`cargo doc --open`). Tempo is best-effort (MIDI, ABC and PriMus carry it,
MusicXML and MEI currently don't). ChordPro and RTTTL are a different kind of
format entirely (chords-over-lyrics with no notated rhythm, and a monophonic
ringtone melody, respectively) so converting to/from them is inherently lossy
in ways the notation-first formats aren't - see their crate docs before
relying on round-tripping through them.

### Notation support

What each plugin reads and writes beyond pitches and rhythms. ✓ = read and
write, ~ = partial (see the crate docs), W = write only, – = not expressible
in the format. "unrolled" means repeats/endings/jumps are played out into a
linear performance on write (via `scores_core::playback_order`).

| Feature                    | MusicXML | MEI | ABC | MuseScore | PriMus    | Guitar Pro       | MIDI                   | ChordPro           | RTTTL    |
|----------------------------|:--------:|:---:|:---:|:---------:|:---------:|:----------------:|:----------------------:|:------------------:|:--------:|
| Dynamics                   | ✓        | ✓   | ✓   | ✓         | ~         | ~ (velocity)     | ✓ (velocity)           | ~ (`<mf>` tokens)  | –        |
| Hairpins                   | ✓        | ✓   | ✓   | ✓         | ✓         | –                | W (velocity ramp)      | –                  | –        |
| Articulations              | ✓        | ✓   | ~   | ✓         | ~         | ~                | W (performed), ~ read  | –                  | –        |
| Ornaments                  | ✓        | ✓   | ~   | ~         | ~         | ~                | W (played out)         | –                  | –        |
| Fermata                    | ✓        | ✓   | ✓   | ✓         | ✓         | –                | ✓ (tempo dip)          | ~ (`<ferm>` token) | –        |
| Grace notes                | ✓        | ✓   | ✓   | ✓         | ✓         | ~                | W                      | –                  | –        |
| Chord symbols              | ✓        | ✓   | ✓   | ✓         | ~         | ✓                | ✓ (text events)        | ✓                  | –        |
| 1st/2nd endings            | ✓        | ✓   | ~   | ✓         | ✓         | ~                | unrolled               | –                  | unrolled |
| Barline style              | ✓        | ~   | ~   | ✓         | ~         | ~ (double only)  | –                      | –                  | –        |
| D.C./D.S./Coda/Segno/Fine  | ✓        | ✓   | ✓   | ✓         | ✓ (texts) | ~ (GP5 only)     | unrolled               | ✓ (`{comment}`)    | unrolled |

## CLI

```sh
cargo run -p scores-cli -- list-formats
cargo run -p scores-cli -- convert input.abc output.musicxml
cargo run -p scores-cli -- convert score.mid output.xml --to musicxml
```

The format is inferred from each file's extension unless `--from`/`--to` is given.

## Web (WASM)

`scores-web` exposes `convert(bytes, from, to)` and `list_formats()` via
`wasm-bindgen`. Building it requires [`wasm-pack`](https://rustwasm.github.io/wasm-pack/)
(`cargo install wasm-pack`, or see its install docs):

```sh
cd crates/scores-web
wasm-pack build --target web --out-dir www/pkg
```

Then serve `crates/scores-web/www/` with any static file server (it must be
served over HTTP(S), not opened as a `file://` URL) and open `index.html`:

```sh
python3 -m http.server -d crates/scores-web/www 8000
```

## Tests

```sh
cargo test --workspace
```
