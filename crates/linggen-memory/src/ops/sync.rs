//! What a replica checks against its primary before it mixes rows in.

/// The embedding model this store's vectors come from.
pub fn model_id() -> &'static str {
    crate::embed::MODEL_REPO
}
