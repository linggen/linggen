//! The curated model list: one signed bundle, read in one place.
//!
//! The owner edits a list on the site (`catalog/curated.json`); a script joins
//! public facts to it and signs the exact bytes. This module is the only
//! reader: [`bundle`] checks and parses (size, signature, schema, revision),
//! [`store`] keeps the last good copy and asks for a newer one. The engine
//! ships the same bytes the site serves, so an engine that has never been
//! online reads the shipped copy through the very same check.
//!
//! Slice 1 of the curated list: nothing in the settings page reads this yet
//! (slice 2 replaces the preset list and the v1 facts with it), so its
//! accessors are used by tests only until then.
#![allow(dead_code, unused_imports)]

mod bundle;
mod store;
mod trust;
#[cfg(test)]
mod tests;

pub(crate) use bundle::{Bundle, Edition};
pub(crate) use store::{current, shipped, start};
