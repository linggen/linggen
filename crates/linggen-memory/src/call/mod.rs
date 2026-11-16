//! [`Memory::call`] — memory's one JSON entry point: a verb's name and its
//! args, run through the verb table (`ops::verbs`).

mod reply;

pub use reply::Reply;

use crate::error::MemError;
use crate::handle::Memory;
use crate::ops::verbs;
use serde_json::Value;
use std::sync::Arc;

impl Memory {
    /// Run the verb `verb` with `args` (the JSON body its route takes), as
    /// a request to its route would: the activity clock is stamped, then
    /// the verb runs on this store.
    ///
    /// The caller's person is whatever the args carry, as the engine's door
    /// stamped them; `call` adds no identity of its own. The next stage adds
    /// a required caller.
    pub async fn call(self: &Arc<Self>, verb: &str, args: Value) -> Result<Reply, MemError> {
        self.mark_request();
        let verb = find(verb)?;
        (verb.run)(Arc::clone(self), args).await.map(Reply::Done)
    }
}

fn find(name: &str) -> Result<&'static verbs::Verb, MemError> {
    verbs::find(name).ok_or_else(|| MemError::not_found(format!("no memory verb {name}")))
}
