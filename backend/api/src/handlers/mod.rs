pub mod chat;
pub mod clear_data;
pub mod enhance;
pub mod graph;
pub mod index;
pub mod index_source;
pub mod intent;
pub mod internal_rescan;
pub mod jobs;
pub mod mcp;
pub mod memory;
pub mod memory_semantic;
pub mod notes;
pub mod preferences;
pub mod profile;
pub mod prompts;
pub mod resources;
pub mod retry_init;
pub mod settings;
pub mod source_memory;
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

pub use internal_rescan::rescan_internal_index;
pub use memory::{create_memory, delete_memory, list_memories, read_memory, update_memory};
pub use memory_semantic::search_semantic as memory_search_semantic;
pub use prompts::{delete_prompt, get_prompt, list_prompts, rename_prompt, save_prompt};
pub use resources::{
    add_resource, list_resources, remove_resource, rename_resource, update_resource_patterns,
};
pub use retry_init::retry_init;
pub use source_memory::{
    delete_memory_file, get_memory_file, list_memory_files, rename_memory_file, save_memory_file,
};
pub mod search;
pub use search::search;
pub use status::get_app_status;
pub use upload::{delete_uploaded_file, list_uploaded_files, upload_file, upload_file_stream};
