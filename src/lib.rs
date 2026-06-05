//! Library entry point for the `rep` crate.
//!
//! The binary in `main.rs` is a thin shim around this library.

pub(crate) mod app;
pub mod cli;
pub(crate) mod document;
pub(crate) mod document_view;
pub(crate) mod markdown;
pub(crate) mod output;
pub(crate) mod selection;
#[cfg(test)]
pub(crate) mod test_support;
pub mod ui;
