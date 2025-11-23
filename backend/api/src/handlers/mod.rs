pub mod enhance;
pub mod index;
pub mod index_folder;
pub mod index_source;
pub mod intent;
pub mod jobs;
pub mod preferences;
pub mod resources;
pub mod search;

pub use enhance::enhance_prompt;
pub use index::{index_document, AppState};
pub use index_folder::index_folder;
pub use index_source::index_source;
pub use intent::classify_intent;
pub use jobs::{cancel_job, list_jobs};
pub use preferences::{get_preferences, update_preferences};
pub use resources::{add_resource, list_resources, remove_resource};
pub use search::search;
