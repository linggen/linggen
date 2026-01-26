mod client;
mod commands;
mod download;
mod installer;
mod jobs;
mod signature;
pub mod skills;
mod util;
pub use util::find_server_binary;

pub use client::ApiClient;
pub use commands::{
    handle_check, handle_doctor, handle_index, handle_install, handle_start, handle_status,
    handle_update,
};
pub use skills::handle_skills_add;
