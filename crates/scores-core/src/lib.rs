//! Shared data model and format plugin traits for the `scores` workspace.
//!
//! Format plugin crates (e.g. `scores-format-musicxml`) implement
//! [`Format`] for their own unit struct, converting to and from [`Score`].
//! Binaries such as `scores-cli` and `scores-web` depend on whichever format
//! crates they want to support and register them into a [`FormatRegistry`].

mod chord_symbol;
mod error;
mod format;
mod model;
mod unroll;

pub use chord_symbol::{ChordKind, ChordRoot, ChordSymbol};
pub use error::{Result, ScoresError};
pub use format::{Format, FormatReader, FormatRegistry, FormatWriter};
pub use model::*;
pub use unroll::playback_order;
