//! [`Memory::call`] — memory's one JSON entry point: a verb's name and its
//! args, for a caller ([`Who`]), run through the verb table (`ops::verbs`),
//! with a replica's writes going to its primary.

mod reply;
mod route;
mod who;

pub use reply::Reply;
pub(crate) use route::{route_of, Route};
pub use who::{machine_only, stamp, Person, Who};

use crate::error::MemError;
use crate::handle::Memory;
use crate::ops::verbs::{self, Verb};
use crate::replica::forward;
use serde_json::Value;
use std::sync::Arc;

impl Memory {
    /// Run the verb `verb` with `args` (the JSON body its route takes) for
    /// `who`: fitted to the caller first ([`Who::scope`] — a person's call
    /// refused, or stamped with the person), then as a request to its route
    /// would run: the activity clock is stamped, and
    ///
    /// - on a primary (or a store on its own) the verb runs here;
    /// - on a replica a read runs here, a primary-only verb is refused
    ///   (`REPLICA`, 409), and a write goes to the primary as these raw args
    ///   (anything queued earlier first; a pull after a success) — or, when
    ///   the primary does not answer, runs here and is queued for it — the
    ///   stamped args, so the primary scopes them as this store would.
    pub async fn call(
        self: &Arc<Self>,
        who: &Who,
        verb: &str,
        mut args: Value,
    ) -> Result<Reply, MemError> {
        who.scope(verb, &mut args)?;
        self.run(verb, args).await
    }

    /// [`call`](Self::call) for args already fitted to their caller.
    pub(crate) async fn run(self: &Arc<Self>, verb: &str, args: Value) -> Result<Reply, MemError> {
        self.mark_request();
        let verb = find(verb)?;
        // Before forwarding or applying a write, including updates that
        // only patch metadata: a call waits for the warm-up up to its bound
        // (the automatic recall, `skip_if_warming`, is answered at once);
        // past it, refused — nothing applied or queued. This does not load
        // a quiet CLI store.
        if matches!(verb.name, "search" | "add" | "add_batch" | "update") {
            if let Some(error) = self.warming(patience_of(&args)).await {
                return Err(error);
            }
        }
        match route::route(self, verb) {
            Route::Here => (verb.run)(Arc::clone(self), args).await.map(Reply::Done),
            Route::Refuse => Err(forward::primary_only(self)),
            Route::Forward => forward_or_queue(self, verb, args).await,
        }
    }
}

/// How a call waits for the warm-up: the automatic recall says
/// `skip_if_warming` and is answered at once; every other call waits.
fn patience_of(args: &Value) -> crate::embed::Patience {
    match args.get("skip_if_warming").and_then(Value::as_bool) {
        Some(true) => crate::embed::Patience::AnswerNow,
        _ => crate::embed::Patience::WaitForWarmup,
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
    // Applied here; not queued means the primary will never hear of it.
    forward::queue(state, verb.path, args, &data).map_err(|e| {
        MemError::internal(anyhow::anyhow!(
            "applied on this machine but not queued for the primary: {e:#}"
        ))
    })?;
    Ok(Reply::Queued(data))
}
