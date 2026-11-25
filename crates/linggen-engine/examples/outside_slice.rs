//! Prints what the memory-v2 forwarding would hand a session: the index
//! lines of the OTHER agents' memories for a folder, by the engine's own
//! code (`own_memory::forward::session_start`, the other agents' part),
//! read-only. Used by
//! `scripts/memory-v2-compare.py`; writes nothing and calls no model.
//!
//!   cargo run -q --example outside_slice -- --cwd <folder> [--host claude-code|codex]

use linggen_engine::outside_memory::{host_agent, Env};
use linggen_engine::own_memory::forward::session_start;
use std::path::PathBuf;

fn main() {
    let mut cwd: Option<PathBuf> = None;
    let mut host_word: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--cwd" => cwd = args.next().map(PathBuf::from),
            "--host" => host_word = args.next(),
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
    let host = host_word.as_deref().and_then(|w| host_agent(&env, w));
    let got = session_start(&env, None, host.as_ref(), cwd.as_deref(), 0);
    print!("{}", got.block);
    for note in &got.others.notes {
        eprintln!("skipped {}: {}", note.source, note.state);
    }
}
