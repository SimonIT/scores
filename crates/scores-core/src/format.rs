use crate::error::Result;
use crate::model::Score;

/// Parses raw bytes of a specific file format into the common [`Score`] model.
pub trait FormatReader {
    fn read(&self, data: &[u8]) -> Result<Score>;
}

/// Serializes a [`Score`] into the raw bytes of a specific file format.
pub trait FormatWriter {
    fn write(&self, score: &Score) -> Result<Vec<u8>>;
}

/// A pluggable score format: identity metadata plus read/write support.
///
/// Format plugins are ordinary crates that implement this trait for a unit
/// struct and are wired into a [`FormatRegistry`] at compile time by whatever
/// binary wants to support them (`scores-cli`, `scores-web`, ...). There is
/// no runtime/dynamic loading involved, which keeps every format usable from
/// both native binaries and WASM.
pub trait Format: FormatReader + FormatWriter {
    /// Stable, lowercase identifier, e.g. `"musicxml"`.
    fn id(&self) -> &'static str;
    /// Human-readable name, e.g. `"MusicXML"`.
    fn name(&self) -> &'static str;
    /// File extensions (without the leading dot) recognized for this format.
    fn extensions(&self) -> &'static [&'static str];
}

/// A collection of [`Format`] implementations, looked up by id or file
/// extension. Binaries build one of these at startup by registering whichever
/// format crates they were compiled with.
#[derive(Default)]
pub struct FormatRegistry {
    formats: Vec<Box<dyn Format>>,
}

impl FormatRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, format: Box<dyn Format>) -> &mut Self {
        self.formats.push(format);
        self
    }

    pub fn by_id(&self, id: &str) -> Option<&dyn Format> {
        self.formats.iter().find(|f| f.id() == id).map(Box::as_ref)
    }

    pub fn by_extension(&self, extension: &str) -> Option<&dyn Format> {
        let extension = extension.trim_start_matches('.');
        self.formats
            .iter()
            .find(|f| {
                f.extensions()
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(extension))
            })
            .map(Box::as_ref)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Format> {
        self.formats.iter().map(Box::as_ref)
    }

    pub fn len(&self) -> usize {
        self.formats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.formats.is_empty()
    }
}
