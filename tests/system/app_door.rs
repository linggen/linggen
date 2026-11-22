//! An outside agent uses an app through the MCP door's one `app` tool
//! (doc/mcp-spec.md § The app tool): the same list and the same judge a
//! session's `AppTool` uses, for the owner at this Mac alone. A private app
//! is closed to it until the owner opens it in Settings.
//!
//! Every app here is a made-up fixture installed by its test.

use crate::app_index::{action, install_app, install_app_with, install_jukebox};
use crate::support::{Setup, World};
use serde_json::{json, Value};

fn rpc(name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

/// A call's text, and whether it was a tool error.
async fn door(w: &World, name: &str, args: Value) -> (String, bool) {
    let res = w.api.post("/mcp", rpc(name, args)).await;
    said(&res, name)
}

fn said(res: &Value, name: &str) -> (String, bool) {
    let text = res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: {res}"))
        .to_string();
    (text, res["result"]["isError"] == json!(true))
}

/// The door's `app` tool, op `list`, for `app` or for every app.
async fn list(w: &World, app: Option<&str>) -> (String, bool) {
    let args = match app {
        Some(app) => json!({"op": "list", "app": app}),
        None => json!({"op": "list"}),
    };
    door(w, "app", args).await
}

async fn call(w: &World, app: &str, action: &str, args: Value) -> (String, bool) {
    door(
        w,
        "app",
        json!({"op": "call", "app": app, "action": action, "args": args}),
    )
    .await
}

/// What the setting holds: the apps the owner opened to outside agents.
async fn open_apps(w: &World, apps: &[&str]) -> (u16, Value) {
    w.api
        .put_json("/api/mcp/outside-apps", json!({"apps": apps}))
        .await
}

/// A private app with one read action offered anywhere.
async fn install_vault(w: &World) {
    let tools = action("Balance", "read", true, "echo balance is 42");
    install_app_with(
        w,
        "vault",
        "Keeps the money.",
        "private-talk: true\n",
        &tools,
    )
    .await;
}

/// This machine's door publishes thirteen tools, `app` last, and none of
/// them a file or shell tool.
#[tokio::test]
async fn the_door_has_thirteen_tools_and_no_file_or_shell_tool() {
    let w = World::start(Setup::default()).await;
    let res = w
        .api
        .post(
            "/mcp",
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .await;
    let names: Vec<&str> = res["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 13, "{names:?}");
    assert_eq!(names.last(), Some(&"app"), "{names:?}");
    for tool in [
        "Read", "Write", "Edit", "Bash", "Glob", "Grep", "WebFetch", "Task", "Skill", "AppTool",
    ] {
        assert!(!names.contains(&tool), "{tool} is at the door");
    }
    let init = w
        .api
        .post(
            "/mcp",
            json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {}}),
        )
        .await;
    let text = init["result"]["instructions"].as_str().unwrap();
    assert!(
        text.contains("app op=list"),
        "the server text says what app is for: {text}"
    );
    assert!(
        text.chars().count() <= 2048,
        "{} characters",
        text.chars().count()
    );
}

/// `list` names each app with an action an outside agent may use, and its
/// actions; with `app` it gives each action with its class, description
/// and arguments — what a session's `AppTool` lists.
#[tokio::test]
async fn app_list_names_the_apps_and_their_actions() {
    let w = World::start(Setup::default()).await;
    let (none, err) = list(&w, None).await;
    assert!(!err && none.contains("No app"), "{none}");

    install_jukebox(&w, "jukebox").await;
    install_app(
        &w,
        "radio",
        "Only plays in its own page.",
        &action("Tune", "read", false, "echo tuned"),
    )
    .await;
    let (all, err) = list(&w, None).await;
    assert!(!err, "{all}");
    assert!(
        all.contains("jukebox") && all.contains("Plays music on this Mac."),
        "{all}"
    );
    assert!(
        all.contains("Actions: List, Play."),
        "only what it may run is named: {all}"
    );
    assert!(!all.contains("Delete"), "{all}");
    assert!(
        !all.contains("radio") && !all.contains("Tune"),
        "an app that offers nothing anywhere is not listed: {all}"
    );

    let (rows, err) = list(&w, Some("jukebox")).await;
    assert!(!err, "{rows}");
    let rows: Vec<Value> = rows
        .lines()
        .map(|l| serde_json::from_str(l).expect(l))
        .collect();
    let tools: Vec<&str> = rows.iter().map(|r| r["tool"].as_str().unwrap()).collect();
    assert_eq!(
        tools,
        ["List", "Play"],
        "a destructive action is not listed"
    );
    assert_eq!(rows[1]["class"], "edit");
    assert_eq!(rows[1]["description"], "The Play action.");
    assert_eq!(rows[1]["args"]["what"], "string?");

    let (why, err) = list(&w, Some("nothing")).await;
    assert!(err && why.contains("no skill 'nothing'"), "{why}");
}

/// A read and an edit action run under the app's own grant and return
/// their result; an action the app keeps to itself does not run.
#[tokio::test]
async fn app_call_runs_an_edit_action_and_returns_its_result() {
    let w = World::start(Setup::default()).await;
    install_jukebox(&w, "jukebox").await;
    let (said, err) = call(&w, "jukebox", "List", json!({})).await;
    assert!(!err && said.contains("listing"), "{said}");
    let (said, err) = call(&w, "jukebox", "Play", json!({"what": "jazz"})).await;
    assert!(!err && said.contains("playing jazz"), "{said}");
    let (why, err) = call(&w, "jukebox", "Tune", json!({})).await;
    assert!(
        err && why == "'jukebox' does not offer 'Tune' to you",
        "{why}"
    );
    let (why, err) = call(&w, "jukebox", "Nope", json!({})).await;
    assert!(err && why == "'jukebox' declares no tool 'Nope'", "{why}");
}

/// A destructive action is refused with the reason, an admin one and one
/// with no class too — and none of them ran.
#[tokio::test]
async fn a_destructive_admin_or_unclassed_action_is_refused_and_does_not_run() {
    let w = World::start(Setup::default()).await;
    let marker = |name: &str| w.home.linggen_home().join(format!("skills/crate/{name}"));
    let tools = [
        action("Delete", "destructive", true, "touch deleted"),
        action("Root", "admin", true, "touch rooted"),
    ]
    .concat()
        + "  - name: Plain\n    description: No class.\n    cmd: \"touch plain\"\n    reach: anywhere\n";
    install_app(&w, "crate", "Holds things.", &tools).await;

    // An app left with no action an outside agent may run has no line.
    let (all, _) = list(&w, None).await;
    assert!(all.contains("No app") && !all.contains("crate"), "{all}");
    let (rows, _) = list(&w, Some("crate")).await;
    assert!(rows.contains("offers an outside agent no action"), "{rows}");

    let (why, err) = call(&w, "crate", "Delete", json!({})).await;
    assert!(
        err && why.contains("the person does this themselves, in the app"),
        "{why}"
    );
    for refused in ["Root", "Plain"] {
        let (why, err) = call(&w, "crate", refused, json!({})).await;
        assert!(err && why.contains("needs admin"), "{refused}: {why}");
    }
    for file in ["deleted", "rooted", "plain"] {
        assert!(!marker(file).exists(), "{file} ran");
    }
}

/// A private app is not in the list and a call is refused, naming the
/// setting; the owner opens it, and it is listed and runs; closing it
/// closes it again.
#[tokio::test]
async fn a_private_app_is_closed_until_the_owner_opens_it() {
    let w = World::start(Setup::default()).await;
    install_vault(&w).await;
    install_jukebox(&w, "jukebox").await;

    let (all, _) = list(&w, None).await;
    assert!(all.contains("jukebox") && !all.contains("vault"), "{all}");
    for (said, err) in [
        list(&w, Some("vault")).await,
        call(&w, "vault", "Balance", json!({})).await,
    ] {
        assert!(
            err && said.contains("Settings") && said.contains("Tools"),
            "{said}"
        );
    }

    let (status, body) = open_apps(&w, &["vault"]).await;
    assert_eq!(status, 200, "{body}");
    let (all, _) = list(&w, None).await;
    assert!(
        all.contains("vault") && all.contains("Keeps the money."),
        "{all}"
    );
    let (said, err) = call(&w, "vault", "Balance", json!({})).await;
    assert!(!err && said.contains("balance is 42"), "{said}");

    let (status, _) = open_apps(&w, &[]).await;
    assert_eq!(status, 200);
    let (why, err) = call(&w, "vault", "Balance", json!({})).await;
    assert!(err && why.contains("Settings"), "{why}");
}

/// The setting lists the private apps that offer an action anywhere,
/// each with whether it is open; it takes only those; and a saved config
/// from a stale page does not move it.
#[tokio::test]
async fn the_setting_lists_the_private_apps_and_is_not_moved_by_a_config_save() {
    let w = World::start(Setup::default()).await;
    install_vault(&w).await;
    install_jukebox(&w, "jukebox").await;
    install_app_with(
        &w,
        "diary",
        "Private, offers nothing.",
        "private-talk: true\n",
        &action("Peek", "read", false, "echo"),
    )
    .await;

    let listed = w.api.get("/api/mcp/outside-apps").await;
    let apps = listed["apps"].as_array().unwrap();
    assert_eq!(apps.len(), 1, "{listed}");
    assert_eq!(
        (apps[0]["name"].as_str(), apps[0]["open"].as_bool()),
        (Some("vault"), Some(false))
    );
    assert_eq!(listed["at_this_mac"], true);

    for bad in [&["jukebox"][..], &["nothing"][..], &["diary"][..]] {
        let (status, _) = open_apps(&w, bad).await;
        assert_eq!(
            status, 400,
            "{bad:?} is not a private app with an action offered anywhere"
        );
    }
    assert_eq!(open_apps(&w, &["vault"]).await.0, 200);
    assert_eq!(
        w.api.get("/api/mcp/outside-apps").await["apps"][0]["open"],
        true
    );

    // A page opened before the change posts its old config back.
    let mut config = w.api.get("/api/config").await;
    assert_eq!(config["outside_apps"], json!(["vault"]));
    config["outside_apps"] = json!([]);
    w.api.post("/api/config", config).await;
    assert_eq!(
        w.api.get("/api/config").await["outside_apps"],
        json!(["vault"])
    );
}

/// Anything but the owner at this Mac gets nothing: a tunnelled owner
/// (the owner's other device) is refused before the app is looked up, and
/// can no more set the setting.
#[tokio::test]
async fn a_caller_that_is_not_this_mac_gets_nothing() {
    let w = World::start(Setup::default()).await;
    install_jukebox(&w, "jukebox").await;
    let base = format!("http://127.0.0.1:{}", w.engine.port);
    let client = reqwest::Client::new();
    let tunnelled = |path: &str| {
        client
            .post(format!("{base}{path}"))
            .header("x-linggen-tunnel", "owner")
    };
    let ask = |args: Value| {
        let req = tunnelled("/mcp").json(&rpc("app", args));
        async move { req.send().await.unwrap().json::<Value>().await.unwrap() }
    };
    let on_a_real_app = ask(json!({"op": "call", "app": "jukebox", "action": "List"})).await;
    let on_no_app = ask(json!({"op": "call", "app": "nothing", "action": "List"})).await;
    let (a, err) = said(&on_a_real_app, "app");
    assert!(err && a.contains("this Mac"), "{a}");
    assert_eq!(
        said(&on_no_app, "app").0,
        a,
        "the answer does not say whether the app exists"
    );
    let (listed, err) = said(&ask(json!({"op": "list"})).await, "app");
    assert!(err && !listed.contains("jukebox"), "{listed}");

    install_vault(&w).await;
    let put = client
        .put(format!("{base}/api/mcp/outside-apps"))
        .header("x-linggen-tunnel", "owner")
        .json(&json!({"apps": ["vault"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status().as_u16(), 403);
    assert!(
        w.api.get("/api/config").await.get("outside_apps").is_none(),
        "nothing was set"
    );
}
