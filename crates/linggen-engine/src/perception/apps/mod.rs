//! One line per app, declared by its skill (`doc/perception-spec.md` §2).
//!
//! The engine names no app. A skill says, in its frontmatter, which small
//! file in its own folder holds its reading and which fields of it make its
//! line ([`declare`]); the engine reads that file when it builds the world and
//! renders one line ([`read`]). The skill loader keeps the list of declaring
//! skills here, so a turn can read it without waiting on the loader.

pub mod declare;
pub mod read;

#[cfg(test)]
mod tests;

pub use declare::WorldLine;
pub use read::App;

use std::sync::Mutex;

static DECLARED: Mutex<Vec<App>> = Mutex::new(Vec::new());
static CACHE: Mutex<Option<read::Cache>> = Mutex::new(None);

/// The skills that declare a line, as the loader last saw them.
pub fn declare(apps: Vec<App>) {
    *DECLARED.lock().unwrap_or_else(|e| e.into_inner()) = apps;
}

#[cfg(test)]
thread_local! {
    /// A test's own list, so tests that load skills elsewhere in the process
    /// cannot change what this thread's world reads.
    static TEST_APPS: std::cell::RefCell<Option<Vec<App>>> = const { std::cell::RefCell::new(None) };
}

/// Make this thread's world read `apps` instead of the loader's list.
#[cfg(test)]
pub(crate) fn declare_for_this_thread(apps: Vec<App>) {
    TEST_APPS.with(|t| *t.borrow_mut() = Some(apps));
}

fn declared() -> Vec<App> {
    #[cfg(test)]
    if let Some(apps) = TEST_APPS.with(|t| t.borrow().clone()) {
        return apps;
    }
    DECLARED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Every declaring app's line, now.
pub fn lines() -> Vec<String> {
    let apps = declared();
    if apps.is_empty() {
        return Vec::new();
    }
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    read::lines(
        &apps,
        std::time::SystemTime::now(),
        g.get_or_insert_with(Default::default),
    )
}
