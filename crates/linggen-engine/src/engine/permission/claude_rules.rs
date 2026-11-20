//! The `deny` and `ask` rules a person wrote in Claude Code's settings
//! (`~/.claude/settings.json`, a project's `.claude/settings.json` and
//! `.claude/settings.local.json`), honoured in a coding session as one more
//! source feeding the one permission check (`model.rs`,
//! `check_permission_under_rules`; permission-spec.md § Claude Code's
//! rules). They only take away: `allow` is never read — a file shipped
//! inside a project must never widen what a session may do, and the
//! person's own allows were given to another program.
//!
//! This module is the one place that reads Claude's rule syntax ([`parse`])
//! and says whether a call matches ([`ClaudeCall::denies`],
//! [`ClaudeCall::asks`]); the check decides what a match does.

use super::model::{command_run, commands_as_written, display_path, file_name, normalize_path};
use std::path::{Component, Path, PathBuf};

/// A settings file larger than this is not read (its rules are listed as
/// not understood): a project file is untrusted input.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// The first words of every refusal a deny rule makes. The gate shows a
/// refusal that starts with them in full ([`is_refusal`]).
const REFUSAL_HEAD: &str = "Refused by Claude Code settings:";

/// Whether a `Blocked` reason is a deny rule's refusal, which names the
/// rule and its file and is shown as it is.
pub fn is_refusal(reason: &str) -> bool {
    reason.starts_with(REFUSAL_HEAD)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Deny,
    Ask,
}

/// The tools a rule names, in Linggen's names.
#[derive(Debug, Clone)]
enum Tools {
    Bash,
    /// `Read(...)`: Read, Glob and Grep.
    Reads,
    /// `Edit(...)` / `Write(...)` / `MultiEdit` / `NotebookEdit`: Edit and Write.
    Edits,
    /// One tool by its exact name (`Glob`, `WebFetch`, `Task`, …).
    One(String),
    /// `mcp__server` or `mcp__server__tool`: the tool, or every tool of
    /// the server.
    Mcp(String),
    /// A name with `*` in it (`*`, `mcp__*`): every tool whose name it matches.
    NameGlob(String),
}

impl Tools {
    fn covers(&self, tool: &str) -> bool {
        match self {
            Tools::Bash => tool == "Bash",
            Tools::Reads => matches!(tool, "Read" | "Glob" | "Grep"),
            Tools::Edits => matches!(tool, "Edit" | "Write"),
            Tools::One(name) => tool == name,
            Tools::Mcp(name) => tool == name || tool.starts_with(&format!("{name}__")),
            Tools::NameGlob(glob) => wild_match(glob, tool),
        }
    }
}

/// What, of a call of the named tools, a rule matches.
#[derive(Debug, Clone)]
enum Matcher {
    /// Every call.
    Every,
    /// A Bash command, as written in the rule (`*` stands for any text).
    Command(String),
    /// A file path, gitignore style, under an anchor.
    Path(PathPattern),
    /// A fetch's host (`domain:`).
    Domain(String),
}

#[derive(Debug, Clone)]
struct PathPattern {
    anchor: PathBuf,
    glob: globset::GlobSet,
}

/// What one rule's text reads as. The one place the syntax is judged.
enum Parsed {
    Understood(Tools, Matcher),
    /// A form not mapped exactly. For a tool Linggen has, a deny refuses
    /// every call of it; an ask is not used. Either way it is listed.
    NotUnderstood(Option<Tools>),
}

#[derive(Debug, Clone)]
struct Rule {
    kind: Kind,
    text: String,
    file: PathBuf,
    tools: Tools,
    matcher: Matcher,
}

/// Where a settings file's rules are anchored.
struct Source {
    file: PathBuf,
    /// Where `/path` starts: `~/.claude` for the person's own file, the
    /// folder holding `.claude/` for a project's (Claude Code's "relative
    /// to the settings source").
    anchor: PathBuf,
    /// Where `path` and `./path` start: the folder the run starts in, or
    /// the git root for the root's own files (it holds that folder).
    base: PathBuf,
}

/// The rules in force for one run: read at its start, so a change the run
/// makes to the files does not take a rule away mid-run.
#[derive(Debug, Clone, Default)]
pub struct ClaudeRules {
    rules: Vec<Rule>,
    not_understood: Vec<String>,
    home: Option<PathBuf>,
}

/// The rules and what of a call they need beyond the check's own inputs.
#[derive(Clone, Copy)]
pub struct ClaudeCall<'a> {
    rules: &'a ClaudeRules,
    url: Option<&'a str>,
}

/// One call, as the check sees it.
pub(super) struct Facts<'a> {
    pub tool: &'a str,
    /// The file or folder a file tool acts on, resolved.
    pub path: Option<PathBuf>,
    /// Every text the command runs as: the whole command and, for a coding
    /// session, the commands it gives a shell as text.
    pub commands: Vec<String>,
    /// Bash: the gate's path targets plus every word that could name a
    /// file, resolved against the folder the command runs in.
    pub bash_paths: Vec<PathBuf>,
    /// Bash: the command is more than a read (an `Edit` rule judges it).
    pub bash_writes: bool,
}

static NONE: ClaudeRules = ClaudeRules {
    rules: Vec::new(),
    not_understood: Vec::new(),
    home: None,
};

impl ClaudeRules {
    /// No rules: the check exactly as it is without Claude's settings.
    pub fn none() -> ClaudeCall<'static> {
        ClaudeCall {
            rules: &NONE,
            url: None,
        }
    }

    /// Read the person's file under `home` and the project's two files
    /// in `project` (and at its git root, when that is another folder).
    /// Only `permissions.deny` and `permissions.ask` are read.
    pub fn load(home: Option<&Path>, project: &Path) -> Self {
        let mut sources = Vec::new();
        if let Some(home) = home {
            let dir = home.join(".claude");
            sources.push(Source {
                file: dir.join("settings.json"),
                anchor: dir,
                base: project.to_path_buf(),
            });
        }
        let mut folders = vec![project.to_path_buf()];
        if let Some(root) = crate::engine::tools::search_exec_find_git_root(project) {
            if normalize_path(&root.to_string_lossy()) != normalize_path(&project.to_string_lossy())
            {
                folders.push(root);
            }
        }
        for folder in folders {
            for name in ["settings.json", "settings.local.json"] {
                sources.push(Source {
                    file: folder.join(".claude").join(name),
                    anchor: folder.clone(),
                    base: folder.clone(),
                });
            }
        }
        let mut rules = ClaudeRules {
            home: home.map(Path::to_path_buf),
            ..Default::default()
        };
        for source in &sources {
            rules.read_source(source);
        }
        rules
    }

    fn read_source(&mut self, source: &Source) {
        let Ok(meta) = std::fs::metadata(&source.file) else {
            return;
        };
        let shown = display_path(&source.file);
        let value = (meta.len() <= MAX_FILE_BYTES)
            .then(|| std::fs::read_to_string(&source.file).ok())
            .flatten()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        let Some(value) = value else {
            self.not_understood.push(format!("{shown} (unreadable)"));
            return;
        };
        let permissions = value.get("permissions");
        for (kind, key) in [(Kind::Deny, "deny"), (Kind::Ask, "ask")] {
            let Some(list) = permissions.and_then(|p| p.get(key)) else {
                continue;
            };
            let Some(list) = list.as_array() else {
                self.not_understood.push(format!("{key} in {shown}"));
                continue;
            };
            for entry in list {
                match entry.as_str() {
                    Some(text) => self.add(kind, text.trim(), source),
                    None => self.not_understood.push(format!("{entry} in {shown}")),
                }
            }
        }
    }

    fn add(&mut self, kind: Kind, text: &str, source: &Source) {
        let parsed = parse(text, &source.anchor, self.home.as_deref(), &source.base);
        let (tools, matcher) = match (parsed, kind) {
            (Parsed::Understood(tools, matcher), _) => (tools, matcher),
            (Parsed::NotUnderstood(tools), kind) => {
                self.not_understood
                    .push(format!("`{text}` in {}", display_path(&source.file)));
                match (tools, kind) {
                    (Some(tools), Kind::Deny) => (tools, Matcher::Every),
                    _ => return,
                }
            }
        };
        self.rules.push(Rule {
            kind,
            text: text.to_string(),
            file: source.file.clone(),
            tools,
            matcher,
        });
    }

    /// The call as the check judges it: `url` is a fetch's address.
    pub fn for_call<'a>(&'a self, url: Option<&'a str>) -> ClaudeCall<'a> {
        ClaudeCall { rules: self, url }
    }

    /// The line shown when a run starts, when any rule was not understood.
    /// Every rule not understood, as the line would name it uncut.
    #[cfg(test)]
    pub(super) fn not_understood(&self) -> &[String] {
        &self.not_understood
    }

    /// At most three are named; the line says how many more there are.
    pub fn notice(&self) -> Option<String> {
        const NAMED: usize = 3;
        if self.not_understood.is_empty() {
            return None;
        }
        let mut named: Vec<String> = self
            .not_understood
            .iter()
            .take(NAMED)
            .map(|rule| clip(rule))
            .collect();
        if self.not_understood.len() > NAMED {
            named.push(format!("{} more", self.not_understood.len() - NAMED));
        }
        Some(format!(
            "Claude Code rules not understood: {}. An unclear deny refuses its whole tool; an unclear ask is not used.",
            named.join(", ")
        ))
    }
}

impl ClaudeCall<'_> {
    /// No rule to judge: the check is exactly as without the files.
    pub(super) fn is_empty(&self) -> bool {
        self.rules.rules.is_empty()
    }

    /// The refusal of the first deny rule the call matches.
    pub(super) fn denies(&self, facts: &Facts<'_>) -> Option<String> {
        self.first(Kind::Deny, facts).map(|rule| {
            format!(
                "{REFUSAL_HEAD} `{}` in {} denies this.",
                rule.text,
                display_path(&rule.file)
            )
        })
    }

    /// The reason of the first ask rule the call matches.
    pub(super) fn asks(&self, facts: &Facts<'_>) -> Option<String> {
        self.first(Kind::Ask, facts).map(|rule| {
            format!(
                "`{}` in {} asks before this, every time.",
                rule.text,
                display_path(&rule.file)
            )
        })
    }

    fn first(&self, kind: Kind, facts: &Facts<'_>) -> Option<&Rule> {
        self.rules
            .rules
            .iter()
            .find(|rule| rule.kind == kind && self.matches(rule, facts))
    }

    fn matches(&self, rule: &Rule, facts: &Facts<'_>) -> bool {
        if rule.tools.covers(facts.tool) {
            return match &rule.matcher {
                Matcher::Every => true,
                Matcher::Command(pattern) => facts.commands.iter().any(|cmd| {
                    command_texts(cmd)
                        .iter()
                        .any(|text| command_match(pattern, text))
                }),
                Matcher::Path(pattern) => facts.path.as_deref().is_some_and(|p| pattern.matches(p)),
                // An address the gate cannot read a host from: matched.
                Matcher::Domain(domain) => self
                    .url
                    .and_then(host_of)
                    .is_none_or(|host| domain_match(domain, &host)),
            };
        }
        // A file rule also judges the files a shell command names.
        let judges_shell = match rule.tools {
            Tools::Reads => true,
            Tools::Edits => facts.bash_writes,
            _ => false,
        };
        match &rule.matcher {
            Matcher::Path(pattern) if facts.tool == "Bash" && judges_shell => {
                facts.bash_paths.iter().any(|p| pattern.matches(p))
            }
            _ => false,
        }
    }
}

/// Read one rule's text. `anchor` is where its file's `/path` starts.
fn parse(text: &str, anchor: &Path, home: Option<&Path>, project: &Path) -> Parsed {
    let (name, spec) = match text.split_once('(') {
        Some((name, rest)) => match rest.strip_suffix(')') {
            Some(spec) => (name.trim(), Some(spec.trim())),
            None => return Parsed::NotUnderstood(tools_named(name.trim())),
        },
        None => (text, None),
    };
    let Some(tools) = tools_named(name) else {
        return Parsed::NotUnderstood(None);
    };
    let spec = match spec {
        None | Some("*") => return Parsed::Understood(tools, Matcher::Every),
        Some("") => return Parsed::NotUnderstood(Some(tools)),
        Some(spec) => spec,
    };
    let matcher = match &tools {
        Tools::Bash
            if !names_a_parameter(
                spec,
                &["command", "description", "timeout", "run_in_background"],
            ) =>
        {
            Some(Matcher::Command(match spec.strip_suffix(":*") {
                Some(head) => format!("{head} *"),
                None => spec.to_string(),
            }))
        }
        // A `!` exception only narrows the rules before it: left out, they
        // stay whole — the strict side — and it is listed.
        Tools::Reads | Tools::Edits if spec.starts_with('!') => return Parsed::NotUnderstood(None),
        Tools::Reads | Tools::Edits => path_pattern(spec, anchor, home, project).map(Matcher::Path),
        Tools::One(name) if matches!(name.as_str(), "Glob" | "Grep") => {
            path_pattern(spec, anchor, home, project).map(Matcher::Path)
        }
        Tools::One(name) if name == "WebFetch" => spec
            .strip_prefix("domain:")
            .map(|d| Matcher::Domain(d.trim().trim_end_matches('.').to_ascii_lowercase())),
        _ => None,
    };
    match matcher {
        Some(matcher) => Parsed::Understood(tools, matcher),
        None => Parsed::NotUnderstood(Some(tools)),
    }
}

/// A rule's tool name in Linggen's names; `None` for a tool Linggen has not.
fn tools_named(name: &str) -> Option<Tools> {
    if name.contains('*') {
        return Some(Tools::NameGlob(name.to_string()));
    }
    if name.starts_with("mcp__") {
        return Some(Tools::Mcp(name.to_string()));
    }
    match name {
        "Bash" => Some(Tools::Bash),
        "Read" => Some(Tools::Reads),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => Some(Tools::Edits),
        "Agent" | "Task" => Some(Tools::One("Task".into())),
        _ if crate::engine::tools::builtin_tier(name).is_some() => Some(Tools::One(name.into())),
        _ => None,
    }
}

/// `Tool(param:value)`: a rule on one of the tool's inputs, not mapped.
fn names_a_parameter(spec: &str, params: &[&str]) -> bool {
    spec.split_once(':')
        .is_some_and(|(head, _)| params.contains(&head))
}

/// A `Read`/`Edit` pattern: gitignore style, anchored as Claude Code
/// anchors it — `//x` the filesystem's root, `~/x` the home folder, `/x`
/// the settings source, anything else the project folder. A name with no
/// `/`, and a single folder's `name/**`, match at any depth below it.
fn path_pattern(
    spec: &str,
    anchor: &Path,
    home: Option<&Path>,
    project: &Path,
) -> Option<PathPattern> {
    if names_a_parameter(spec, &["file_path", "path", "pattern"]) {
        return None;
    }
    let (anchor, rel, anywhere) = if let Some(rel) = spec.strip_prefix("//") {
        (PathBuf::from("/"), rel, false)
    } else if spec == "~" {
        (home?.to_path_buf(), "", false)
    } else if let Some(rel) = spec.strip_prefix("~/") {
        (home?.to_path_buf(), rel, false)
    } else if let Some(rel) = spec.strip_prefix('/') {
        (anchor.to_path_buf(), rel, false)
    } else {
        let rel = spec.strip_prefix("./").unwrap_or(spec);
        let bare = rel.trim_end_matches('/');
        let one_folder = bare
            .strip_suffix("/**")
            .is_some_and(|dir| !dir.contains('/'));
        (
            project.to_path_buf(),
            rel,
            !bare.contains('/') || one_folder,
        )
    };
    let rel = rel.trim_end_matches('/');
    let glob = match (rel.is_empty(), anywhere) {
        (true, _) => "**".to_string(),
        (false, true) => format!("**/{rel}"),
        (false, false) => rel.to_string(),
    };
    // `name/**` also matches the folder itself (a search rooted there).
    let mut globs = globset::GlobSetBuilder::new();
    for glob in [Some(glob.as_str()), glob.strip_suffix("/**")]
        .into_iter()
        .flatten()
    {
        // A Mac's disk does not tell `.ENV` from `.env`.
        let mut builder = globset::GlobBuilder::new(glob);
        builder
            .literal_separator(true)
            .backslash_escape(true)
            .case_insensitive(true);
        globs.add(builder.build().ok()?);
    }
    Some(PathPattern {
        anchor,
        glob: globs.build().ok()?,
    })
}

impl PathPattern {
    /// Whether `target`, or a folder it is in, matches below the anchor —
    /// read both as written and with links resolved.
    fn matches(&self, target: &Path) -> bool {
        let pairs = [
            (lexical(target), lexical(&self.anchor)),
            (
                PathBuf::from(normalize_path(&target.to_string_lossy())),
                PathBuf::from(normalize_path(&self.anchor.to_string_lossy())),
            ),
        ];
        pairs.iter().any(|(target, anchor)| {
            let Ok(rel) = target.strip_prefix(anchor) else {
                return false;
            };
            let mut prefix = PathBuf::new();
            rel.components().any(|part| {
                prefix.push(part);
                self.glob.is_match(&prefix)
            }) || (rel.as_os_str().is_empty() && self.glob.is_match(""))
        })
    }
}

/// `path` with `.` and `..` collapsed, without asking the disk.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Every form of `cmd` a Bash rule is matched against: the whole text,
/// each command in it (split as the shell and as the floor split it), and
/// each command as it really runs — past `NAME=value` words, the programs
/// that only run what follows (Claude Code's `timeout`, `stdbuf`, `noglob`
/// among them), and its program by its file name.
fn command_texts(cmd: &str) -> Vec<String> {
    let mut texts = vec![cmd.trim().to_string()];
    texts.extend(
        super::model::split_command_segments(cmd)
            .into_iter()
            .map(|seg| seg.trim().to_string()),
    );
    for words in commands_as_written(cmd) {
        texts.push(words.join(" "));
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        let mut run = command_run(&refs);
        loop {
            let Some(first) = run.first() else { break };
            let program = file_name(first);
            if !matches!(program.as_str(), "timeout" | "stdbuf" | "noglob") {
                break;
            }
            let mut rest = &run[1..];
            while rest.first().is_some_and(|w| w.starts_with('-')) {
                rest = &rest[1..];
            }
            if program == "timeout" && !rest.is_empty() {
                rest = &rest[1..];
            }
            run = command_run(rest);
        }
        if let Some(first) = run.first() {
            texts.push(run.join(" "));
            let mut named = vec![file_name(first)];
            named.extend(run[1..].iter().map(|w| w.to_string()));
            texts.push(named.join(" "));
        }
    }
    texts.retain(|text| !text.is_empty());
    texts.dedup();
    texts
}

/// A Bash rule's pattern against one command: `*` stands for any text, and
/// a pattern ending in ` *` as its only wildcard also matches the bare
/// command (`git log *` matches `git log`).
fn command_match(pattern: &str, text: &str) -> bool {
    if wild_match(pattern, text) {
        return true;
    }
    match pattern.strip_suffix(" *") {
        Some(head) if !head.contains('*') => head == text,
        _ => false,
    }
}

/// `*` matches any run of characters; everything else matches itself.
fn wild_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() || !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// A rule's text as shown in a line: a project file is untrusted, so a
/// long one is cut.
fn clip(text: &str) -> String {
    const MOST: usize = 80;
    match text.char_indices().nth(MOST) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn host_of(url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
    Some(host.trim_end_matches('.').to_string())
}

/// A `domain:` pattern against a host: `*` is every host, `*.x` any
/// subdomain of x, and any other `*` stands for the text of one label.
fn domain_match(pattern: &str, host: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(parent) = pattern.strip_prefix("*.") {
        return host.ends_with(&format!(".{parent}"));
    }
    let (want, have): (Vec<&str>, Vec<&str>) =
        (pattern.split('.').collect(), host.split('.').collect());
    want.len() == have.len() && want.iter().zip(&have).all(|(w, h)| wild_match(w, h))
}
