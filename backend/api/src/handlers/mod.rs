pub mod chat;
pub mod clear_data;
pub mod enhance;
pub mod graph;
pub mod index;
pub mod index_source;
pub mod intent;
pub mod jobs;
pub mod mcp;
pub mod notes;
pub mod preferences;
pub mod profile;
pub mod resources;
pub mod retry_init;
pub mod settings;
pub mod status;
pub mod upload;

pub use chat::chat_stream;
pub use clear_data::clear_all_data;
pub use enhance::enhance_prompt;
pub use graph::{get_graph, get_graph_status, get_graph_with_status, rebuild_graph};
pub use index::AppState;
pub use index_source::index_source;
pub use intent::classify_intent;
pub use jobs::{cancel_job, list_jobs};
// pub use preferences::{get_preferences, update_preferences};

pub use resources::{
    add_resource, list_resources, remove_resource, rename_resource, update_resource_patterns,
};
pub use retry_init::retry_init;
pub mod search;
pub use search::search;
pub use status::get_app_status;
pub use upload::{delete_uploaded_file, list_uploaded_files, upload_file, upload_file_stream};
