//! Library entry point for the `rep` crate.
//!
//! The binary in `main.rs` is a thin shim around this library. Integration
//! tests in `tests/` import from here.

pub mod app;
pub mod cli;
pub(crate) mod document;
pub(crate) mod document_view;
pub(crate) mod markdown;
pub(crate) mod output;
pub(crate) mod selection;
pub mod ui;
