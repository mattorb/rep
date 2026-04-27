//! Library entry point for the `rep` crate.
//!
//! The binary in `main.rs` is a thin shim around this library. Integration
//! tests in `tests/` import from here.

pub mod app;
pub mod cli;
pub mod document;
pub mod markdown;
pub mod output;
pub mod selection;
pub mod ui;
