//! Theme data model, parsers, and resolution.
//!
//! `model` defines the validated value types (`HexColor`, `ThemeName`) and the
//! parsed shapes (`ParsedPalette`, `ParsedRc`). `parse` turns the two on-disk
//! formats (`.colorant` palettes and `.colorantrc` configs) into those shapes.
//! `resolve` flattens a `ParsedRc` into the single `ThemeLayer` to emit for a
//! given mode, walking the `extends` chain along the way.

pub(crate) mod bundled;
pub mod gogh;
pub mod model;
pub mod parse;
pub mod rc;
pub mod resolve;
pub mod source;
