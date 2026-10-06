//! The hermetic system-test harness (doc/test-design.md § 2).
//!
//! A [`World`] is one test's whole Linggen: the fixture home copied into a
//! fresh root, a fake model, a scratch ling-mem and the built engine — each
//! on its own port, none of them the person's. Dropping it stops everything
//! and removes the root.

pub mod api;
pub mod engine;
pub mod guard;
pub mod home;
pub mod memory;
pub mod model;
pub mod process;
pub mod rows;

use api::Api;
use engine::{Engine, Mark, RunEnd};
use home::Home;
use memory::Memory;
use model::{Agent, Extra, FakeModel, Reply};

pub use model::{text, tool};

pub struct World {
    pub api: Api,
    pub engine: Engine,
    pub model: FakeModel,
    pub memory: Memory,
    // Last: removed after every process above has stopped.
    pub home: Home,
}

/// What a world starts with beyond the fixture home.
#[derive(Default)]
pub struct Setup {
    pub scripts: Vec<(Agent, Vec<Reply>)>,
    pub extras: Vec<Extra>,
    /// Load the real embedding model in ling-mem (slow; few tests).
    pub embedder: bool,
    /// Seed this `tests/fixtures/memory/` file before the engine starts
    /// (needs `embedder`).
    pub memory_rows: Option<&'static str>,
    /// The `tests/fixtures/<dir>` the home is copied from; `home` when unset
    /// (`fresh` is a new install: no models, no sessions, no skills).
    pub fixture: Option<&'static str>,
}

impl Setup {
    pub fn script(mut self, agent: Agent, replies: Vec<Reply>) -> Self {
        self.scripts.push((agent, replies));
        self
    }

    pub fn extra(mut self, system_contains: &'static str, reply: &'static str) -> Self {
        self.extras.push(Extra {
            system_contains,
            reply,
        });
        self
    }
}

impl World {
    pub async fn start(setup: Setup) -> Self {
        let home = Home::new();
        let model = FakeModel::start(&setup.scripts, &setup.extras).await;
        let memory = Memory::start(&home, setup.embedder).await;
        let port = Engine::new_port();
        let mut vars = home.base_vars();
        vars.insert("ENGINE_PORT", port.to_string());
        vars.insert("MEM_URL", memory.url.clone());
        vars.insert("MODEL_URL", model.url());
        home.install_fixtures(setup.fixture.unwrap_or("home"), &vars);
        let config = std::fs::read_to_string(home.config_path()).expect("config");
        guard::config_text(&config);
        guard::config_mem_url(&config, &memory.url);
        guard::home_files(&home.home(), &home.model_cache());
        if let Some(file) = setup.memory_rows {
            memory.seed(file, &vars).await;
        }
        let engine = Engine::start(&home, port).await;
        let api = Api::new(engine.base_url());
        Self {
            api,
            engine,
            model,
            memory,
            home,
        }
    }

    /// The plain folder ordinary chats start in.
    pub fn work(&self) -> String {
        self.home.work().display().to_string()
    }

    pub fn mark(&self) -> Mark {
        self.engine.mark()
    }

    /// Wait for `agent`'s run in `sid`, begun after `mark`.
    pub async fn wait_run(&self, sid: &str, agent: Agent, mark: Mark) -> RunEnd {
        let api = self.api.clone();
        let who = agent.id();
        self.engine
            .wait_run(Some(sid), who, mark, || {
                let api = api.clone();
                let sid = sid.to_string();
                async move { api.pending(&sid, Some(who)).await }
            })
            .await
    }

    /// Wait for `agent`'s next run in whatever session it runs (her daily
    /// thread, say), begun after `mark`.
    pub async fn wait_agent(&self, agent: Agent, mark: Mark) {
        let end = self
            .engine
            .wait_run(None, agent.id(), mark, || async { Vec::new() })
            .await;
        assert!(matches!(end, RunEnd::Finished), "{end:?}");
    }

    /// Wait for a run that must finish without asking anything.
    pub async fn finish_run(&self, sid: &str, agent: Agent, mark: Mark) {
        if let RunEnd::Asked(ask) = self.wait_run(sid, agent, mark).await {
            panic!(
                "{}'s run in {sid} asked instead of finishing: {ask}",
                agent.id()
            );
        }
    }

    /// Send `message` in `sid` and wait for `agent`'s run to finish.
    pub async fn turn(
        &self,
        sid: &str,
        root: &str,
        message: &str,
        agent: Agent,
    ) -> serde_json::Value {
        let mark = self.mark();
        let resp = self.api.chat(sid, root, message).await;
        assert_eq!(resp["agent_id"], agent.id(), "routed elsewhere: {resp}");
        self.finish_run(sid, agent, mark).await;
        resp
    }

    /// The rows of a session's thread on disk (it must be there).
    pub fn rows(&self, sid: &str) -> Vec<rows::Row> {
        rows::read(&self.home.linggen_home(), sid)
    }

    /// A session's rows, or `None` when it has no thread on disk.
    pub fn rows_opt(&self, sid: &str) -> Option<Vec<rows::Row>> {
        rows::read_opt(&self.home.linggen_home(), sid)
    }

    /// Fail when any model call went unscripted (it would make a test pass
    /// on a reply nobody chose).
    pub fn assert_all_scripted(&self) {
        let stray = self.model.unscripted();
        if stray.is_empty() {
            return;
        }
        let what: Vec<String> = stray
            .iter()
            .map(|c| {
                format!(
                    "{} model={} last={:?}",
                    c.path,
                    c.model(),
                    c.thread().last()
                )
            })
            .collect();
        panic!(
            "{} unscripted model call(s):\n{}",
            stray.len(),
            what.join("\n")
        );
    }
}

impl Drop for World {
    /// Whatever the test did, the engine and the model never heard of the
    /// real engine or ling-mem (checked on a passing test; a failing one
    /// already says why).
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        guard::after_run("the engine log", &self.engine.log());
        for call in self.model.calls() {
            guard::after_run(&format!("a model call to {}", call.path), &call.text());
        }
    }
}

/// Snapshot a model call's thread (what the engine built for that member),
/// with the world's paths, ids and clock filtered out.
pub fn snap(name: &str, w: &World, call: &model::Call) {
    let mut out = String::new();
    for (role, body) in call.thread() {
        out.push_str(&format!("── {role}\n{}\n", body.trim_end()));
    }
    snap_text(name, w, &out);
}

/// Snapshot any text from a world, filtered the same way.
pub fn snap_text(name: &str, w: &World, text: &str) {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/system/snapshots"
    ));
    settings.set_prepend_module_to_snapshot(false);
    let root = regex::escape(&w.home.root.display().to_string());
    let root = root.trim_start_matches("/private");
    settings.add_filter(&format!("(/private)?{root}"), "[ROOT]");
    settings.add_filter(r"sess-\d+-[0-9a-f]{8}", "[SID]");
    settings.add_filter(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})",
        "[TIME]",
    );
    settings.add_filter(r"\b\d{4}-\d{2}-\d{2}\b", "[DATE]");
    settings.add_filter(r"\b\d{1,2}:\d{2}(:\d{2})?\b", "[CLOCK]");
    settings.add_filter(r"call_[A-Za-z0-9_-]+", "[CALL_ID]");
    // The machine the test runs on.
    settings.add_filter(
        r"(?m)^- (Platform|OS|Timezone|Locale): .*$",
        "- $1: [MACHINE]",
    );
    settings.bind(|| insta::assert_snapshot!(name, text));
}
