//! Release locks over the repository: the catalogue is the only place an
//! MCP tool is defined or served, and what describes or calls the door —
//! the plugin for both hosts, its evals, the scripts that install and gate
//! it, the docs of the live door, the engine's own agents and missions —
//! names no retired tool, no retired address and no old namespace, and
//! sends no old shape. A clearly separated upgrade fixture (a path with
//! `upgrade-fixture`) and a history section of a doc (a heading with
//! "History" or "Retired") may.

use super::catalog::catalog;
use ling_mem::catalog::View;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.')
            && name != ".claude-plugin"
            && name != ".codex-plugin"
            && name != ".mcp.json"
        {
            continue;
        }
        if ["node_modules", "target", "__pycache__", "results"].contains(&name.as_str()) {
            continue;
        }
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Every live file that describes or calls the door.
fn live_files() -> Vec<PathBuf> {
    let root = root();
    let mut files = Vec::new();
    for dir in [
        "plugins/linggen",
        "evals",
        "scripts/plugin",
        "scripts/gate",
        "agents",
        "missions",
        "prompts",
    ] {
        walk(&root.join(dir), &mut files);
    }
    for file in [
        "scripts/live-check.sh",
        ".claude-plugin/marketplace.json",
        "README.md",
        "doc/mcp-spec.md",
        "doc/mcp-client-spec.md",
        "doc/memory-spec.md",
        "doc/network-spec.md",
        "doc/cli.md",
        "doc/permission-spec.md",
        "doc/test-design.md",
        "doc/release-checklist.md",
        "doc/browser-control-spec.md",
    ] {
        files.push(root.join(file));
    }
    let text = |p: &PathBuf| {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        ["md", "sh", "json", "py", "toml", "yaml", "yml", "txt", ""].contains(&ext)
    };
    files.retain(|p| text(p) && !p.to_string_lossy().contains("upgrade-fixture"));
    files
}

/// Whole-word: `browser_read` is a tool, `browser_read_page` retired.
fn names(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(at, _)| {
        let part = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !part(line[..at].chars().next_back()) && !part(line[at + word.len()..].chars().next())
    })
}

/// Lines of `text` with whether they stand in a history section.
fn lines_with_history(path: &Path, text: &str) -> Vec<(usize, String, bool)> {
    let markdown = path.extension().is_some_and(|e| e == "md");
    let mut headings: Vec<(usize, bool)> = Vec::new();
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if markdown && line.starts_with('#') {
            let level = line.chars().take_while(|c| *c == '#').count();
            headings.retain(|(l, _)| *l < level);
            let lower = line.to_lowercase();
            headings.push((
                level,
                lower.contains("history") || lower.contains("retired"),
            ));
        }
        let history = headings.iter().any(|(_, h)| *h);
        out.push((n + 1, line.to_string(), history));
    }
    out
}

/// No live file names a retired tool, the retired memory address or the
/// old memory server's namespace, nor sends `agent_status` without its op;
/// the retired address and namespace are named only where the 410 is
/// described, or a line beside them says they are retired (a check's own
/// pattern list).
#[test]
fn no_live_file_names_a_retired_tool_address_or_namespace() {
    let files = live_files();
    assert!(files.len() > 40, "found only {} files", files.len());
    let retired: Vec<&str> = catalog().retired().iter().map(|r| r.name).collect();
    let mut stale = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let shown = path
            .strip_prefix(root())
            .unwrap_or(path)
            .display()
            .to_string();
        let lines = lines_with_history(path, &text);
        for (i, (n, line, history)) in lines.iter().enumerate() {
            let (n, line, history) = (*n, line.as_str(), *history);
            if history {
                continue;
            }
            // A line says what the retired address is when it, or a line
            // beside it, names its answer or calls it retired.
            let near = &lines[i.saturating_sub(2)..(i + 3).min(lines.len())];
            for name in &retired {
                if names(line, name) {
                    stale.push(format!("{shown}:{n}: {name}"));
                }
            }
            let describes_410 = near.iter().any(|(_, l, _)| {
                ["410", "UPGRADE_PLUGIN", "retired", "Retired", "RETIRED"]
                    .iter()
                    .any(|w| l.contains(w))
            });
            if line.contains("plugin_linggen_memory") && !describes_410 {
                stale.push(format!("{shown}:{n}: the old memory namespace"));
            }
            if line.contains("/mcp/memory") && !describes_410 {
                stale.push(format!("{shown}:{n}: /mcp/memory"));
            }
            for (at, _) in line.match_indices("\"agent_status\"") {
                let after: String = line[at..].chars().take(160).collect();
                if after.contains("session_id") && !after.contains("op") {
                    stale.push(format!("{shown}:{n}: agent_status without op"));
                }
            }
        }
    }
    assert!(
        stale.is_empty(),
        "stale names in live files:\n{}",
        stale.join("\n")
    );
}

/// The only code that defines an MCP tool (writes an `inputSchema`) is the
/// catalogue's schema module, and the only servers that answer
/// `tools/list` are the two built from a catalogue: this door and memory's
/// own for the engine's agents. Third-party servers the engine connects to
/// as a client are not this check's.
#[test]
fn only_the_catalogue_defines_and_serves_mcp_tools() {
    let root = root();
    let mut files = Vec::new();
    for dir in [
        "src",
        "crates/linggen-engine/src",
        "crates/linggen-memory/src",
    ] {
        walk(&root.join(dir), &mut files);
    }
    files.retain(|p| p.extension().is_some_and(|e| e == "rs"));
    assert!(files.len() > 100, "found only {} files", files.len());
    let defines = ["crates/linggen-memory/src/catalog/schema.rs"];
    let serves = [
        "src/server/mcp/mod.rs",
        "crates/linggen-memory/src/http/mcp.rs",
    ];
    let mut wrong = Vec::new();
    for path in &files {
        let shown = path.strip_prefix(&root).unwrap().display().to_string();
        if shown.ends_with("tests.rs")
            || shown.contains("/tests/")
            || shown.ends_with("repo_check.rs")
        {
            continue;
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        if code.contains("\"inputSchema\":") && !defines.contains(&shown.as_str()) {
            wrong.push(format!("{shown} defines an MCP tool"));
        }
        if code.contains("\"tools/list\" =>") && !serves.contains(&shown.as_str()) {
            wrong.push(format!("{shown} serves tools/list"));
        }
        if serves.contains(&shown.as_str()) && !code.contains("tools_list(") {
            wrong.push(format!("{shown} serves a list that is not the catalogue's"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The catalogue is exactly twelve tools, and the list and the dispatch are
/// the same table: every (tool, op) a view lists resolves to exactly that
/// branch, and every branch a view may call is listed — no branch is
/// reachable that the list does not show, and none listed runs nothing.
#[test]
fn the_list_and_the_dispatch_are_one_table() {
    let cat = catalog();
    assert_eq!(cat.tools().len(), 12);
    for view in [View::Local, View::Remote] {
        let mut listed = Vec::new();
        for tool in cat.tools_list(view) {
            let name = tool["name"].as_str().unwrap().to_string();
            match tool["inputSchema"]["properties"]["op"]["enum"].as_array() {
                Some(ops) => listed.extend(
                    ops.iter()
                        .map(|op| (name.clone(), op.as_str().unwrap().to_string())),
                ),
                None => listed.push((name.clone(), cat.visible(view, &name)[0].op.to_string())),
            }
        }
        let mut callable: Vec<(String, String)> = cat
            .branches()
            .iter()
            .filter(|b| b.views.has(view))
            .map(|b| (b.tool.to_string(), b.op.to_string()))
            .collect();
        let mut sorted = listed.clone();
        sorted.sort();
        callable.sort();
        assert_eq!(sorted, callable, "{view:?}");
        for (name, op) in &listed {
            let args = if cat.visible(view, name).len() > 1 {
                serde_json::json!({"op": op})
            } else {
                serde_json::json!({})
            };
            let branch = cat
                .branch_of(view, name, &args)
                .unwrap_or_else(|| panic!("{name} {op}"));
            assert_eq!(
                (branch.tool, branch.op),
                (name.as_str(), op.as_str()),
                "{view:?}"
            );
        }
    }
}

/// The plugin evals' tools snapshot is the door's own list for this
/// machine, not a hand-kept copy: a model in an eval is shown exactly what
/// a host would be.
#[test]
fn the_eval_snapshot_is_the_doors_list() {
    let path = root().join("plugins/linggen/evals-isolated/mocks/linggen/_tools.json");
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let door = serde_json::json!({ "tools": catalog().tools_list(View::Local) });
    assert!(
        file == door,
        "{} differs from the door's list; it should read:\nSNAPSHOT-BEGIN\n{}\nSNAPSHOT-END",
        path.display(),
        serde_json::to_string_pretty(&door).unwrap()
    );
}

/// Each eval case is shown this door's server text for this machine, as a
/// host shows its MCP server's instructions — not an older server's.
#[test]
fn the_eval_cases_carry_the_doors_server_text() {
    let text = super::instructions(View::Local);
    let body = format!("# MCP Server Instructions\n\n## plugin:linggen:linggen\n{text}");
    let block: String = body
        .split('\n')
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("  {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut cases = Vec::new();
    walk(&root().join("plugins/linggen/evals-isolated"), &mut cases);
    cases.retain(|p| p.file_name().is_some_and(|n| n == "prompt.md"));
    assert!(cases.len() >= 5, "{cases:?}");
    for case in cases {
        let said = std::fs::read_to_string(&case).unwrap();
        assert!(
            said.contains(&format!("append_system_prompt: |\n{block}\n---")),
            "{} carries another server text",
            case.display()
        );
    }
}

/// What the plugin's hooks send, as their source writes it, is a call the
/// door takes in both views — the schemas are strict, and a hook drops a
/// refused answer silently, so a shape the door refuses would turn recall
/// off without a word. The stamped shapes are stamp-cwd.sh's real output,
/// for every memory branch it may see.
#[test]
fn the_plugins_hook_calls_are_calls_the_door_takes() {
    use ling_mem::catalog::Resolved;
    let hooks = root().join("plugins/linggen/hooks");
    let source = |file: &str| std::fs::read_to_string(hooks.join(file)).unwrap();
    // The literal shapes, tied to the line in each hook that builds them.
    let written = [
        ("autostart.sh", "{op: \"session_start\", cwd: $c}"),
        ("recall.sh", "{query:$q, limit:$l}"),
        ("recall.sh", "'{\"op\":\"days\",\"undreamed_only\":true}'"),
        (
            "recall-on-delegate.sh",
            "{query:$q, limit:$l} + (if ($c | length) > 0 then {scope_root:$c} else {} end)",
        ),
    ];
    for (file, shape) in written {
        assert!(
            source(file).contains(shape),
            "{file} no longer builds {shape}"
        );
    }
    let mut calls = vec![
        (
            "memory_upkeep",
            serde_json::json!({"op": "session_start", "cwd": "/w/p", "root": "/w"}),
        ),
        (
            "memory_search",
            serde_json::json!({"query": "what does Alex drink", "limit": 5, "scope_root": "/w"}),
        ),
        (
            "memory_upkeep",
            serde_json::json!({"op": "days", "undreamed_only": true}),
        ),
        (
            "memory_search",
            serde_json::json!({"query": "a delegated brief", "limit": 3}),
        ),
        // The status command's dream read, this machine's view only below.
        ("memory_upkeep", serde_json::json!({"op": "dream_status"})),
    ];
    // stamp-cwd.sh, run: each call a model makes that the hook stamps, as
    // the hook sends it on (an add with every stamp, a search with its root).
    for (tool, mut args) in memory_calls() {
        let branch = catalog().branch_of(View::Local, tool, &args).unwrap();
        let required = (branch.schema)()["required"].clone();
        for need in required.as_array().into_iter().flatten() {
            let need = need.as_str().unwrap();
            if args.get(need).is_none() {
                args[need] = serde_json::json!("x");
            }
        }
        let sent = stamp_hook(tool, &args);
        if sent != args {
            calls.push((tool, sent));
        }
    }
    assert!(calls.len() >= 9, "the hook stamped nothing: {calls:?}");
    for view in [View::Local, View::Remote] {
        for (tool, args) in &calls {
            match catalog().resolve(view, tool, args.clone()) {
                Resolved::Call { .. } => {}
                Resolved::Refused(why) => panic!("{view:?}: {tool} {args} refused: {why}"),
                Resolved::Unknown => panic!("{view:?}: {tool} unknown"),
            }
        }
    }
}

/// What the plugin's stamp hook (stamp-cwd.sh) sends a memory call out
/// with, run for real: a Claude Code session standing in a project.
fn stamp_hook(tool: &str, args: &serde_json::Value) -> serde_json::Value {
    use std::io::Write;
    let hook = root().join("plugins/linggen/hooks/stamp-cwd.sh");
    let input = serde_json::json!({
        "tool_name": format!("mcp__plugin_linggen_linggen__{tool}"),
        "tool_input": args,
        "cwd": "/work/project",
        "session_id": "s1",
    });
    let mut child = std::process::Command::new("bash")
        .arg(&hook)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", "/nonexistent-home")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("bash runs the hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{tool} {args}: the hook failed");
    let said = String::from_utf8_lossy(&out.stdout);
    if said.trim().is_empty() {
        return args.clone();
    }
    let said: serde_json::Value = serde_json::from_str(&said).unwrap();
    said["hookSpecificOutput"]["updatedInput"].clone()
}

/// Every memory call a host makes, one per branch (and a branch's list
/// that names a day): the arguments a model writes before the hook.
fn memory_calls() -> Vec<(&'static str, serde_json::Value)> {
    let cat = catalog();
    let mut calls = Vec::new();
    for branch in cat.branches() {
        if !branch.tool.starts_with("memory_") {
            continue;
        }
        let mut args = serde_json::json!({});
        if cat.visible(View::Local, branch.tool).len() > 1 {
            args["op"] = serde_json::json!(branch.op);
        }
        if branch.advertises("day") {
            let mut on_a_day = args.clone();
            on_a_day["day"] = serde_json::json!("2026-11-01");
            calls.push((branch.tool, on_a_day));
        }
        calls.push((branch.tool, args));
    }
    // An op-less query: the tool's implicit branch.
    calls.push(("memory_search", serde_json::json!({"query": "q"})));
    calls
}

/// Which memory call gets which stamp is the catalogue's to say (each
/// branch's HOST-FILLED fields, `Branch::host_fills`): the stamp hook adds
/// exactly those, on every branch — a list that names a day included,
/// which is never narrowed to a place.
#[test]
fn the_stamp_hook_adds_what_the_catalogue_says_the_host_fills() {
    let cat = catalog();
    let mut wrong = Vec::new();
    for (tool, args) in memory_calls() {
        let branch = cat
            .branch_of(View::Local, tool, &args)
            .unwrap_or_else(|| panic!("{tool} {args}"));
        let mut want = branch.host_filled(&args);
        let sent = stamp_hook(tool, &args);
        let mut added: Vec<String> = sent
            .as_object()
            .into_iter()
            .flat_map(|o| o.keys().cloned())
            .filter(|k| args.get(k).is_none())
            .collect();
        want.sort();
        added.sort();
        if want != added {
            wrong.push(format!("{tool} {args}: catalogue {want:?}, hook {added:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The names an alternation like `a_(b|c)|d` stands for: `a_b`, `a_c`, `d`.
fn alternatives(pattern: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in pattern.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 => {
                parts.push(&pattern[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&pattern[start..]);
    let mut names = Vec::new();
    for part in parts {
        match part.split_once('(') {
            Some((head, rest)) => {
                let (group, tail) = rest.split_once(')').expect("a closed group");
                names.extend(group.split('|').map(|g| format!("{head}{g}{tail}")));
            }
            None => names.push(part.to_string()),
        }
    }
    names
}

/// plugin-check.sh's own grep for retired names in the plugin is the
/// catalogue's retired names (and the old memory server's namespace),
/// exactly: a name the catalogue retires and that list misses fails here.
#[test]
fn the_plugin_checks_retired_names_are_the_catalogues() {
    let script = std::fs::read_to_string(root().join("scripts/plugin/plugin-check.sh")).unwrap();
    let pattern = script
        .lines()
        .find_map(|l| l.strip_prefix("RETIRED_NAMES='"))
        .and_then(|l| l.strip_suffix('\''))
        .expect("plugin-check.sh sets RETIRED_NAMES='…'");
    let mut listed = alternatives(pattern);
    let mut want: Vec<String> = catalog()
        .retired()
        .iter()
        .map(|r| r.name.to_string())
        .chain(["plugin_linggen_memory".to_string()])
        .collect();
    listed.sort();
    want.sort();
    assert_eq!(listed, want);
    assert_eq!(alternatives("a_(b|c)|d"), ["a_b", "a_c", "d"]);
}

/// What a model is told is the host's (a field described HOST-FILLED) is
/// exactly what the catalogue's stamps say the host fills, branch by
/// branch: the published text and the unpublished table cannot drift.
#[test]
fn the_host_filled_text_is_the_stamps_table() {
    for branch in catalog().branches() {
        let schema = (branch.schema)();
        let mut told: Vec<String> = schema["properties"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, p)| {
                p["description"]
                    .as_str()
                    .is_some_and(|d| d.starts_with(ling_mem::catalog::HOST_FILLED))
            })
            .map(|(f, _)| f.clone())
            .collect();
        let mut stamped: Vec<String> = branch.stamps.iter().map(|s| s.field.to_string()).collect();
        for field in &stamped {
            assert!(branch.advertises(field), "{} {}: stamps {field}", branch.tool, branch.op);
        }
        // A list's scope_root is a filter a caller may set, and the host's
        // when it does not (a day's list excepted): stamped, not HOST-FILLED.
        stamped.retain(|f| {
            branch.stamps.iter().any(|s| s.field == f && s.unless.is_none())
        });
        told.sort();
        stamped.sort();
        assert_eq!(told, stamped, "{} {}", branch.tool, branch.op);
    }
}
