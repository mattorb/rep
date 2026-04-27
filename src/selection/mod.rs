//! Selection / navigation domain.
//!
//! See `modular_plan.md` and `implementation.md` for the architectural
//! contract. This module is the canonical home for selection state, the
//! per-node selection-plain-text view, the eager selection index, the pure
//! navigator, and anchor-to-highlight projection.

pub mod index;
pub mod model;
pub mod navigator;
pub mod projection;
pub mod segment;
