//! [`Memory::call`] — memory's one JSON entry point: a verb's name and its
//! args, run through the verb table (`ops::verbs`), with a replica's writes
//! going to its primary.

mod reply;
mod route;

pub use reply::Reply;
pub(crate) use route::{route_of, Route};

use crate::error::MemError;
use crate::handle::Memory;
use crate::ops::verbs::{self, Verb};
use crate::replica::forward;
use serde_json::Value;
use std::sync::Arc;

impl Memory {
    /// Run the verb `verb` with `args` (the JSON body its route takes), as
    /// a request to its route would: the activity clock is stamped, then
    ///
    /// - on a primary (or a store on its own) the verb runs here;
    /// - on a replica a read runs here, a primary-only verb is refused
    ///   (`REPLICA`, 409), and a write goes to the primary as these raw args
    ///   (anything queued earlier first; a pull after a success) — or, when
    ///   the primary does not answer, runs here and is queued for it.
    ///
    /// The caller's person is whatever the args carry, as the engine's door
    /// stamped them; `call` adds no identity of its own. The next stage adds
    /// a required caller.
    pub async fn call(self: &Arc<Self>, verb: &str, args: Value) -> Result<Reply, MemError> {
        self.mark_request();
        let verb = find(verb)?;
        match route::route(self, verb) {
            Route::Here => (verb.run)(Arc::clone(self), args).await.map(Reply::Done),
            Route::Refuse => Err(forward::primary_only(self)),
            Route::Forward => forward_or_queue(self, verb, args).await,
        }
    }
}

fn find(name: &str) -> Result<&'static Verb, MemError> {
    verbs::find(name).ok_or_else(|| MemError::not_found(format!("no memory verb {name}")))
}

/// A replica's write: the primary's answer, or applied here and queued.
async fn forward_or_queue(
    state: &Arc<Memory>,
    verb: &Verb,
    args: Value,
) -> Result<Reply, MemError> {
    if let Some(answer) = forward::forward_in_order(state, verb.path, &args).await {
        return Ok(answer);
    }
    let data = (verb.run)(Arc::clone(state), args.clone()).await?;
    forward::queue(state, verb.path, args, &data);
    Ok(Reply::Queued(data))
}
