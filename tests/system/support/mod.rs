//! The hermetic system-test harness (doc/test-design.md § 2).
//!
//! A [`World`] is one test's whole Linggen: the fixture home copied into a
//! fresh root, a fake model and the built engine (which hosts memory) — each
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
pub mod voice;

use api::Api;
use engine::{Engine, Mark, RunEnd};
use home::Home;
use memory::Memory;
use model::{Agent, Extra, FakeModel, Reply};

pub use model::{error, say, text, tool};

/// A test that cannot run on this machine (`why`) says so: on stderr, and by
/// name in the file `LINGGEN_TEST_SKIPPED` names, which
/// `scripts/scratch-test.sh` lists as SKIPPED at the end of the run — a pass
/// on nothing is never silently green.
pub fn skipped(test: &str, why: &str) {
    use std::io::Write;
    eprintln!("SKIPPED (passes on nothing): {why}");
    let Some(file) = std::env::var_os("LINGGEN_TEST_SKIPPED") else {
        return;
    };
    // One write per line: tests run in parallel, and an appended write this
    // short lands whole.
    let line = format!("{test} — {why}\n");
    let opened = std::fs::OpenOptions::new().create(true).append(true).open(file);
    if let Ok(mut out) = opened {
        let _ = out.write_all(line.as_bytes());
    }
}

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
    /// Load the real embedding model in the engine's memory (slow; few tests).
    pub embedder: bool,
    /// Seed this `tests/fixtures/memory/` file once the engine is up
    /// (needs `embedder`).
    pub memory_rows: Option<&'static str>,
    /// The `tests/fixtures/<dir>` the home is copied from; `home` when unset.
    pub fixture: Option<&'static str>,
    /// A true first install: no fixture and no config at all — the engine
    /// runs on its own defaults, its port from `--port`. (`fixture` is
    /// ignored.)
    pub no_config: bool,
    /// Where the engine sends a picture request (`LINGGEN_IMAGE_API_URL`):
    /// a loopback stand-in for ChatGPT's image generation.
    pub image_api: Option<String>,
    /// Not signed in to ChatGPT: the fixture's `codex_auth.json` is removed.
    pub signed_out: bool,
    /// The memory the lane gate reads (`LINGGEN_MACHINE_MEMORY_GB`), so a
    /// lane's verdict does not depend on the machine the tests run on.
    pub machine_memory_gb: Option<u64>,
    /// Text appended to the rendered config before the engine starts (an
    /// extra `[[models]]` block, say). Rendered like the fixture files.
    pub config_append: Option<&'static str>,
    /// Leave `[memory]` to the engine's own defaults (v2 since 3.0). Unset:
    /// the world is pinned to the old v1 memory, unless `config_append`
    /// writes a `[memory]` section of its own — most tests are about
    /// something else and were written against v1.
    pub memory_defaults: bool,
    /// More of the engine's environment, set last (a stand-in service's
    /// address, say).
    pub env: Vec<(&'static str, String)>,
    /// The one loopback socket allowed for public web and picture fetches.
    pub fetch_fixture: Option<std::net::SocketAddr>,
    /// Bind the engine to every address (`--host 0.0.0.0`), so a test can
    /// reach it from this machine's own LAN address, as a phone does.
    pub lan: bool,
    /// `routing.default_models` saved as this one model before the engine
    /// starts (the fixture's is `fake-ling`); `None` leaves the fixture's.
    pub default_model: Option<&'static str>,
    /// `agent.daily_session` saved in the config before the engine starts;
    /// `None` leaves the engine's default.
    pub daily_session: Option<bool>,
    /// Rooms switched on (`rooms_on = true` in `room_config.toml`) before the
    /// engine starts; off, as on a machine that never set it, when unset.
    pub rooms_on: bool,
}

impl Setup {
    pub fn script(mut self, agent: Agent, replies: Vec<Reply>) -> Self {
        self.scripts.push((agent, replies));
        self
    }

    /// Linggen's daily session saved off before the engine starts, whatever
    /// the engine's default: every app's chat is a session of its own.
    pub fn setting_off(mut self) -> Self {
        self.daily_session = Some(false);
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

/// Write `agent.daily_session = on` into the rendered config's `[agent]`.
fn save_daily_session(home: &Home, on: bool) {
    let config = std::fs::read_to_string(home.config_path()).expect("config");
    assert!(config.contains("\n[agent]\n"), "the fixture's config has no [agent]");
    let config = config.replacen("\n[agent]\n", &format!("\n[agent]\ndaily_session = {on}\n"), 1);
    std::fs::write(home.config_path(), config).expect("config");
}

/// Write `routing.default_models = [model]` over the fixture's.
fn save_default_model(home: &Home, model: &str) {
    const FIXTURE: &str = "default_models = [\"fake-ling\"]";
    let config = std::fs::read_to_string(home.config_path()).expect("config");
    assert!(config.contains(FIXTURE), "the fixture's config has no default model line");
    let config = config.replacen(FIXTURE, &format!("default_models = [\"{model}\"]"), 1);
    std::fs::write(home.config_path(), config).expect("config");
}

impl World {
    pub async fn start(setup: Setup) -> Self {
        Self::start_with_home(setup, |_| {}).await
    }

    pub async fn start_with_home(setup: Setup, prepare: impl FnOnce(&Home)) -> Self {
        let home = Home::new();
        let model = FakeModel::start(&setup.scripts, &setup.extras).await;
        Memory::prepare(&home, setup.embedder);
        let port = Engine::new_port();
        let mut vars = home.base_vars();
        vars.insert("ENGINE_PORT", port.to_string());
        vars.insert("MODEL_URL", model.url());
        if setup.no_config {
            guard::no_config(&home);
        } else {
            home.install_fixtures(setup.fixture.unwrap_or("home"), &vars);
            if let Some(extra) = setup.config_append {
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(home.config_path())
                    .expect("config");
                std::io::Write::write_all(
                    &mut file,
                    format!("\n{}\n", home::render(extra, &vars)).as_bytes(),
                )
                .expect("append config");
            }
            let chose_memory = setup.config_append.is_some_and(|c| c.contains("[memory]"));
            if !setup.memory_defaults && !chose_memory {
                let mut file = std::fs::OpenOptions::new().append(true).open(home.config_path()).expect("config");
                std::io::Write::write_all(&mut file, b"\n[memory]\nlinggen = \"v1\"\noutside_agents = \"v1\"\n")
                    .expect("append config");
            }
            if let Some(on) = setup.daily_session {
                save_daily_session(&home, on);
            }
            if let Some(model) = setup.default_model {
                save_default_model(&home, model);
            }
            let config = std::fs::read_to_string(home.config_path()).expect("config");
            guard::config_text(&config);
        }
        prepare(&home);
        guard::home_files(&home.home(), &home.model_cache());
        if setup.rooms_on {
            std::fs::write(home.linggen_home().join("room_config.toml"), "rooms_on = true\n")
                .expect("room_config.toml");
        }
        if setup.signed_out {
            let _ = std::fs::remove_file(home.linggen_home().join("codex_auth.json"));
        }
        let mut extra_env: Vec<(String, String)> = setup
            .image_api
            .iter()
            .map(|url| ("LINGGEN_IMAGE_API_URL".to_string(), url.clone()))
            .collect();
        if let Some(gb) = setup.machine_memory_gb {
            extra_env.push(("LINGGEN_MACHINE_MEMORY_GB".to_string(), gb.to_string()));
        }
        extra_env.extend(setup.env.iter().map(|(k, v)| (k.to_string(), v.clone())));
        let host = setup.lan.then_some("0.0.0.0");
        let engine = Engine::start(&home, port, !setup.no_config, &extra_env, host, setup.fetch_fixture).await;
        let memory = Memory::hosted(engine.port);
        if let Some(file) = setup.memory_rows {
            memory.seed(file, &vars).await;
        }
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

    /// Every UI event the engine sent so far (`LINGGEN_EVENT_LOG`), in order.
    pub fn events(&self) -> Vec<serde_json::Value> {
        let text = std::fs::read_to_string(self.home.root.join("events.jsonl")).unwrap_or_default();
        // The engine may be mid-write: a last line without its newline is
        // not an event yet.
        let whole = text.rfind('\n').map_or("", |end| &text[..end]);
        whole
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad event {l}: {e}")))
            .collect()
    }

    /// The events once `seen` holds for them, read again for up to five
    /// seconds: the engine writes its event log from a task of its own, so a
    /// run can end a moment before its last events are on disk. Returns the
    /// last read either way; the caller's own assertion names what is missing.
    pub async fn events_when(
        &self,
        seen: impl Fn(&[serde_json::Value]) -> bool,
    ) -> Vec<serde_json::Value> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let events = self.events();
            if seen(&events) || std::time::Instant::now() >= deadline {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// A session's rows on disk as JSON, every field kept.
    pub fn raw_rows(&self, sid: &str) -> Vec<serde_json::Value> {
        rows::read_raw(&self.home.linggen_home(), sid)
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
    // This machine's own disk, as the world block reads it each turn.
    settings.add_filter(r"(?m)^- this Mac's disk: .*$", "- this Mac's disk: [DISK]");
    settings.bind(|| insta::assert_snapshot!(name, text));
}
