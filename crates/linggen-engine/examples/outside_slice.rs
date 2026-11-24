//! Prints what the memory-v2 forwarding would hand a session: the index
//! lines of the OTHER agents' memories for a folder, by the engine's own
//! code (`outside_memory::slice`), read-only. Used by
//! `scripts/memory-v2-compare.py`; writes nothing and calls no model.
//!
//!   cargo run -q --example outside_slice -- --cwd <folder> [--host claude-code|codex]

use linggen_engine::outside_memory::{slice, Agent, Env};
use std::path::PathBuf;

fn main() {
    let mut cwd: Option<PathBuf> = None;
    let mut host: Option<Agent> = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--cwd" => cwd = args.next().map(PathBuf::from),
            "--host" => host = args.next().as_deref().and_then(Agent::of_host),
            other => {
                eprintln!("unknown flag {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(env) = Env::this_machine() else {
        eprintln!("no home folder");
        std::process::exit(1);
    };
    let got = slice(&env, host, cwd.as_deref());
    print!("{}", got.block());
    for note in &got.notes {
        eprintln!("skipped {}: {}", note.source, note.state);
    }
}
