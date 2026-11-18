//! Activity vocabulary shared by record producers and readers.

/// The activity verb the engine writes for a run that had something go
/// wrong. The engine's own, like `restart`.
pub(crate) const FAULT_VERB: &str = "fault";
