//! WASM bindings exposing score format conversion to the browser.
//!
//! Compiled with `wasm-pack build --target web`, this produces an ES module
//! (`pkg/scores_web.js` + `.wasm`) that `web/index.html` loads directly -
//! see that file for a minimal file-in/file-out demo UI.

use scores_core::FormatRegistry;
use wasm_bindgen::prelude::*;

fn build_registry() -> FormatRegistry {
    let mut registry = FormatRegistry::new();
    registry.register(Box::new(scores_format_musicxml::MusicXmlFormat));
    registry.register(Box::new(scores_format_mei::MeiFormat));
    registry.register(Box::new(scores_format_midi::MidiFormat));
    registry.register(Box::new(scores_format_abc::AbcFormat));
    registry.register(Box::new(scores_format_rtttl::RtttlFormat));
    registry.register(Box::new(scores_format_chordpro::ChordProFormat));
    registry.register(Box::new(scores_format_guitarpro::GuitarProFormat));
    registry.register(Box::new(scores_format_musescore::MuseScoreFormat));
    registry.register(Box::new(scores_format_primus::PrimusFormat));
    registry
}

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Converts `data` from one registered format to another, identified by
/// [`scores_core::Format::id`] (see [`list_formats`]).
#[wasm_bindgen]
pub fn convert(data: &[u8], from: &str, to: &str) -> Result<Vec<u8>, JsValue> {
    let registry = build_registry();

    let from_format = registry
        .by_id(from)
        .ok_or_else(|| JsValue::from_str(&format!("unknown source format '{from}'")))?;
    let to_format = registry
        .by_id(to)
        .ok_or_else(|| JsValue::from_str(&format!("unknown target format '{to}'")))?;

    let score = from_format
        .read(data)
        .map_err(|e| JsValue::from_str(&format!("failed to parse input as {from}: {e}")))?;
    to_format
        .write(&score)
        .map_err(|e| JsValue::from_str(&format!("failed to convert to {to}: {e}")))
}

/// Returns the registered formats as a JSON array of
/// `{ id, name, extensions }` objects, for building a format picker.
#[wasm_bindgen]
pub fn list_formats() -> String {
    let registry = build_registry();
    let entries: Vec<String> = registry
        .iter()
        .map(|format| {
            let extensions = format
                .extensions()
                .iter()
                .map(|e| format!("\"{e}\""))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                r#"{{"id":"{}","name":"{}","extensions":[{}]}}"#,
                format.id(),
                format.name(),
                extensions
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}
