//! The curated model list: one bundle, read in one place.
//!
//! The owner edits a list on the site (`catalog/curated.json`); a script joins
//! public facts to it and publishes the exact bytes. This module is the only
//! reader: [`bundle`] checks and parses (size, schema, revision; no signature is required, the list is fetched over HTTPS),
//! [`store`] keeps the last good copy and asks for a newer one. The engine
//! ships the same bytes the site serves, so an engine that has never been
//! online reads the shipped copy through the very same check.
//!
//! What this engine offers (providers, models, starters), which version a
//! family means, and every public fact about a model all come from here: the
//! one reader. Adding a provider or a model to the list changes no code.

mod bundle;
mod edition;
mod store;
mod transport;
mod trust;
#[cfg(test)]
mod tests;

pub(crate) use bundle::{probe_row, Bundle, Edition, EntryType, Place};
pub use edition::{EditionChoice, ModelCatalog};
pub(crate) use store::{current, start};
#[cfg(test)]
pub(crate) use store::shipped;
