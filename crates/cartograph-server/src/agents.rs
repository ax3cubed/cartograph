//! SRV-21: setting up coding agents (F5, F6). `cartograph agents install | remove | status` and the
//! app's `/api/v1/agents` routes (FE-21) both call these functions.
//!
//! One writer per target file. Each reads the file, changes only the `cartograph` entry or the
//! managed block, and keeps everything else as it was: comments and other keys in JSON with
//! comments (a small span edit, below), TOML through `toml_edit`, markdown outside the markers. A
//! malformed file is refused, never rewritten. Writes go through a temp file and a rename, and the
//! file's previous bytes are kept in one `<file>.bak`.
//!
//! What gets written is fixed text: the MCP entry `{"command": "cartograph", "args": ["mcp"]}` and
//! the instruction block from cli-and-mcp section 6. Nothing read from a repo goes into any of
//! these files (F8).
//!
//! Where (proposed reading of the section 6 table): by default the MCP entry goes in each agent's
//! user config, and Claude Code's block goes in the workspace folder's `CLAUDE.local.md`, which is
//! personal and not committed; `--shared` puts it in `CLAUDE.md` instead. `--project` writes the
//! project-level MCP file and every agent's instruction file in the workspace folder, `--per-repo`
//! the same in each repo; neither is ever a default. With the Claude Code plugin installed, Claude
//! Code gets nothing: the plugin carries the server, the skill and the hook (F6).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use cartograph_api::types::{AgentInstall, AgentStatus, FileDiff};
use cartograph_core::Explain;
use serde_json::{Value, json};

/// The first line of the managed block; a block is found by its first 21 characters.
pub const START: &str = "<!-- cartograph:start · managed by `cartograph agents install` -->";
pub const END: &str = "<!-- cartograph:end -->";
const START_PREFIX: &str = "<!-- cartograph:start";
const BODY: &str = "## Codebase map (Cartograph)
This repo is part of a Cartograph workspace. Use the `cartograph` MCP tools.
- Call `status` first. Each answer says how current the index is.
- Before changing code another service may depend on, call `impact` with the paths.
  Read \"likely\" and \"guessed\" results as leads to check, not facts.
- For uncommitted work, pass `working_tree: true`.
- If a link is wrong, call `correct` with a reason instead of working around it.";
/// Cursor's rule file is ours alone: this front matter, then the block.
const RULE_HEAD: &str = "---\ndescription: Cartograph codebase map\nalwaysApply: true\n---\n";
const KEY: &str = "cartograph";

/// The instruction block, markers included, without a final newline.
pub fn block() -> String {
    format!("{START}\n{BODY}\n{END}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Cursor,
    VsCode,
    Codex,
}

impl Agent {
    pub const ALL: [Agent; 4] = [
        Agent::ClaudeCode,
        Agent::Cursor,
        Agent::VsCode,
        Agent::Codex,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude-code",
            Agent::Cursor => "cursor",
            Agent::VsCode => "vscode",
            Agent::Codex => "codex",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Cursor => "Cursor",
            Agent::VsCode => "VS Code (Copilot)",
            Agent::Codex => "Codex",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// The folder whose presence means the agent was used on this machine.
    fn folder(self, home: &Path) -> PathBuf {
        match self {
            Agent::ClaudeCode => home.join(".claude"),
            Agent::Cursor => home.join(".cursor"),
            Agent::VsCode => vscode_user_dir(home),
            Agent::Codex => home.join(".codex"),
        }
    }

    fn binary(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude",
            Agent::Cursor => "cursor",
            Agent::VsCode => "code",
            Agent::Codex => "codex",
        }
    }

    /// The user-level MCP config.
    pub fn user_config(self, home: &Path) -> PathBuf {
        match self {
            // Claude Code keeps user-scope servers in ~/.claude.json's top-level `mcpServers`.
            Agent::ClaudeCode => home.join(".claude.json"),
            Agent::Cursor => home.join(".cursor/mcp.json"),
            Agent::VsCode => vscode_user_dir(home).join("mcp.json"),
            Agent::Codex => home.join(".codex/config.toml"),
        }
    }

    /// The project-level MCP config inside `dir`.
    fn project_config(self, dir: &Path) -> PathBuf {
        match self {
            Agent::ClaudeCode => dir.join(".mcp.json"),
            Agent::Cursor => dir.join(".cursor/mcp.json"),
            Agent::VsCode => dir.join(".vscode/mcp.json"),
            Agent::Codex => dir.join(".codex/config.toml"),
        }
    }

    fn mcp_format(self) -> Format {
        match self {
            Agent::ClaudeCode | Agent::Cursor => Format::Json {
                parent: "mcpServers",
                stdio: false,
            },
            Agent::VsCode => Format::Json {
                parent: "servers",
                stdio: true,
            },
            Agent::Codex => Format::Toml,
        }
    }
}

fn vscode_user_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Code/User")
    } else if cfg!(windows) {
        home.join("AppData/Roaming/Code/User")
    } else {
        home.join(".config/Code/User")
    }
}

/// `--project`, `--per-repo` and `--shared`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scope {
    pub project: bool,
    pub per_repo: bool,
    pub shared: bool,
}

/// The folders a run may write under: the user's home, and the workspace folder and its repos
/// when there is a workspace.
#[derive(Debug, Clone)]
pub struct Place {
    pub home: PathBuf,
    /// The folder that holds `cartograph.yaml`.
    pub workspace: Option<PathBuf>,
    pub repos: Vec<PathBuf>,
}

/// One file's change: the bytes it has now and the bytes it will have. `None` is no file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub agent: Agent,
    /// Symlinks resolved.
    pub path: PathBuf,
    pub old: Option<String>,
    pub new: Option<String>,
}

impl Change {
    pub fn diff(&self) -> String {
        diff(
            &self.path.to_string_lossy(),
            self.old.as_deref().unwrap_or(""),
            self.new.as_deref().unwrap_or(""),
        )
    }

    pub fn file_diff(&self, applied: bool) -> FileDiff {
        FileDiff {
            file: self.path.to_string_lossy().into_owned(),
            diff: self.diff(),
            applied,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("{path} isn't a file Cartograph can edit safely")]
    Malformed { path: String, why: String },
    #[error("{path} leads outside your home folder and the workspace")]
    Outside { path: String },
    #[error("can't write {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} changed since the preview")]
    Changed { path: String },
    #[error("no workspace covers this folder")]
    NoWorkspace,
    #[error("no agent called `{0}`")]
    UnknownAgent(String),
}

impl Explain for AgentError {
    fn why(&self) -> String {
        match self {
            Self::Malformed { why, .. } => {
                format!("it doesn't parse ({why}), so it was left as it is")
            }
            Self::Outside { .. } => {
                "it is a symlink to a place agents install doesn't write to".into()
            }
            Self::Io { source, .. } => source.to_string(),
            Self::Changed { .. } => "another program wrote it while the change was waiting".into(),
            Self::NoWorkspace => "--project and --per-repo write into a workspace".into(),
            Self::UnknownAgent(_) => "the agents are claude-code, cursor, vscode and codex".into(),
        }
    }

    fn action(&self) -> String {
        match self {
            Self::Malformed { .. } => "fix the file by hand, then run it again".into(),
            Self::Outside { .. } => {
                "replace the symlink with a file, or add the entry by hand".into()
            }
            Self::Io { .. } => "check the folder's permissions".into(),
            Self::Changed { .. } => "run it again to see the new preview".into(),
            Self::NoWorkspace => "run `cartograph init` in the workspace folder first".into(),
            Self::UnknownAgent(_) => "use one of claude-code, cursor, vscode or codex".into(),
        }
    }
}

// ---------------------------------------------------------------- detection

/// The agent's config folder exists, or its binary is on PATH.
pub fn detected(home: &Path, agent: Agent) -> bool {
    agent.folder(home).is_dir() || on_path(agent.binary(), std::env::var_os("PATH").as_deref())
}

fn on_path(bin: &str, path: Option<&OsStr>) -> bool {
    let Some(path) = path else { return false };
    std::env::split_paths(path).any(|d| {
        let names: &[&str] = if cfg!(windows) {
            &[".exe", ".cmd"]
        } else {
            &[""]
        };
        names
            .iter()
            .any(|ext| d.join(format!("{bin}{ext}")).is_file())
    })
}

/// The Claude Code plugin is installed (F6), so Claude Code needs no entry and no block.
pub fn plugin_installed(home: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(home.join(".claude/plugins/installed_plugins.json"))
    else {
        return false;
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("plugins")?
                .as_object()
                .map(|p| p.keys().any(|k| k.starts_with("cartograph@")))
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------- targets

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// JSON with comments: `{ <parent>: { "cartograph": … } }`.
    Json { parent: &'static str, stdio: bool },
    /// `[mcp_servers.cartograph]`.
    Toml,
    /// The managed block in a markdown file.
    Block,
    /// A file that is only ours: front matter and the block.
    Rule,
}

fn targets(agent: Agent, place: &Place, scope: Scope, plugin: bool) -> Vec<(PathBuf, Format)> {
    if agent == Agent::ClaudeCode && plugin {
        return vec![];
    }
    let dirs: Vec<PathBuf> = if scope.per_repo {
        place.repos.clone()
    } else if scope.project {
        place.workspace.iter().cloned().collect()
    } else {
        vec![]
    };
    let mut v = Vec::new();
    if dirs.is_empty() {
        v.push((agent.user_config(&place.home), agent.mcp_format()));
    }
    for d in &dirs {
        v.push((agent.project_config(d), agent.mcp_format()));
        match agent {
            Agent::ClaudeCode => {}
            Agent::Cursor => v.push((d.join(".cursor/rules/cartograph.mdc"), Format::Rule)),
            Agent::VsCode => v.push((d.join(".github/copilot-instructions.md"), Format::Block)),
            Agent::Codex => v.push((d.join("AGENTS.md"), Format::Block)),
        }
    }
    if agent == Agent::ClaudeCode {
        let md = if scope.shared {
            "CLAUDE.md"
        } else {
            "CLAUDE.local.md"
        };
        let block_dirs = if dirs.is_empty() {
            place.workspace.iter().cloned().collect()
        } else {
            dirs
        };
        v.extend(block_dirs.iter().map(|d| (d.join(md), Format::Block)));
    }
    v
}

// ---------------------------------------------------------------- planning and writing

/// The changes an install (or a remove) would make for `agents`, one per file that would change.
/// Reads only.
pub fn plan(
    place: &Place,
    scope: Scope,
    agents: &[Agent],
    install: bool,
) -> Result<Vec<Change>, AgentError> {
    if (scope.project || scope.per_repo) && place.workspace.is_none() {
        return Err(AgentError::NoWorkspace);
    }
    let roots: Vec<PathBuf> = std::iter::once(&place.home)
        .chain(place.workspace.iter())
        .chain(place.repos.iter())
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    // A remove takes the entry out even when the plugin arrived after the install.
    let plugin = install && plugin_installed(&place.home);
    let mut changes = Vec::new();
    for &agent in agents {
        for (path, format) in targets(agent, place, scope, plugin) {
            let real = resolve(&path, &roots)?;
            let old = read(&real)?;
            let malformed = |why| AgentError::Malformed {
                path: real.display().to_string(),
                why,
            };
            let new = if install {
                edit(format, old.as_deref(), true)
            } else {
                removed(format, &real, old.as_deref())
            }
            .map_err(malformed)?;
            if new != old {
                changes.push(Change {
                    agent,
                    path: real,
                    old,
                    new,
                });
            }
        }
    }
    Ok(changes)
}

/// Writes the planned changes. A file that changed since the plan is refused, and each file's
/// previous bytes go to `<file>.bak` first.
pub fn apply(changes: &[Change]) -> Result<(), AgentError> {
    for c in changes {
        if read(&c.path)? != c.old {
            return Err(AgentError::Changed {
                path: c.path.display().to_string(),
            });
        }
        let io = |source| AgentError::Io {
            path: c.path.display().to_string(),
            source,
        };
        if let Some(old) = &c.old {
            write_atomic(&backup(&c.path), old, &c.path).map_err(io)?;
        }
        match &c.new {
            Some(text) => write_atomic(&c.path, text, &c.path).map_err(io)?,
            None => std::fs::remove_file(&c.path).map_err(io)?,
        }
    }
    Ok(())
}

/// Each agent: whether it is on this machine, and whether Cartograph is set up for it (the
/// plugin, the user-level entry, or the workspace folder's entry).
pub fn status(home: &Path, workspace: Option<&Path>) -> Vec<AgentStatus> {
    let plugin = plugin_installed(home);
    let has = |a: Agent, p: &Path| {
        std::fs::read_to_string(p).is_ok_and(|t| {
            edit(a.mcp_format(), Some(&t), false).is_ok_and(|n| n.as_deref() != Some(&t))
        })
    };
    Agent::ALL
        .into_iter()
        .map(|a| {
            let user = a.user_config(home);
            let installed = (a == Agent::ClaudeCode && plugin)
                || has(a, &user)
                || workspace.is_some_and(|w| has(a, &a.project_config(w)));
            AgentStatus {
                id: a.id().into(),
                name: a.name().into(),
                detected: detected(home, a),
                installed,
                config_path: Some(user.to_string_lossy().into_owned()),
            }
        })
        .collect()
}

fn one(
    home: &Path,
    workspace: Option<&Path>,
    req: &AgentInstall,
) -> Result<Vec<Change>, AgentError> {
    let agent =
        Agent::from_id(&req.agent).ok_or_else(|| AgentError::UnknownAgent(req.agent.clone()))?;
    let place = Place {
        home: home.to_owned(),
        workspace: workspace.map(Path::to_owned),
        repos: vec![],
    };
    let scope = Scope {
        shared: req.shared,
        ..Scope::default()
    };
    plan(&place, scope, &[agent], true)
}

/// `POST /api/v1/agents/preview`: the diff of each file an install of one agent would change.
pub fn preview(
    home: &Path,
    workspace: Option<&Path>,
    req: &AgentInstall,
) -> Result<Vec<FileDiff>, AgentError> {
    Ok(one(home, workspace, req)?
        .iter()
        .map(|c| c.file_diff(false))
        .collect())
}

/// `POST /api/v1/agents/install`: the same changes, written.
pub fn install(
    home: &Path,
    workspace: Option<&Path>,
    req: &AgentInstall,
) -> Result<Vec<FileDiff>, AgentError> {
    let changes = one(home, workspace, req)?;
    apply(&changes)?;
    Ok(changes.iter().map(|c| c.file_diff(true)).collect())
}

fn read(p: &Path) -> Result<Option<String>, AgentError> {
    match std::fs::read(p) {
        Ok(b) => String::from_utf8(b)
            .map(Some)
            .map_err(|_| AgentError::Malformed {
                path: p.display().to_string(),
                why: "it isn't UTF-8 text".into(),
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(AgentError::Io {
            path: p.display().to_string(),
            source,
        }),
    }
}

fn backup(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

/// The path with every symlink resolved; it must stay under one of `roots`.
fn resolve(p: &Path, roots: &[PathBuf]) -> Result<PathBuf, AgentError> {
    let mut base = p;
    let mut rest = Vec::new();
    while std::fs::symlink_metadata(base).is_err() {
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                base = parent;
            }
            _ => break,
        }
    }
    let mut real = base.canonicalize().map_err(|source| AgentError::Io {
        path: p.display().to_string(),
        source,
    })?;
    real.extend(rest.iter().rev());
    if roots.iter().any(|r| real.starts_with(r)) {
        Ok(real)
    } else {
        Err(AgentError::Outside {
            path: p.display().to_string(),
        })
    }
}

/// A temp file beside `path`, renamed over it. The new file takes `like`'s permissions when it
/// exists, so `~/.claude.json` stays user-only.
fn write_atomic(path: &Path, text: &str, like: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.cartograph-{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        if let Ok(m) = std::fs::metadata(like) {
            std::fs::set_permissions(&tmp, m.permissions())?;
        }
        std::fs::rename(&tmp, path)
    };
    write().inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

// ---------------------------------------------------------------- the edits

fn entry(stdio: bool) -> Value {
    if stdio {
        json!({ "type": "stdio", "command": "cartograph", "args": ["mcp"] })
    } else {
        json!({ "command": "cartograph", "args": ["mcp"] })
    }
}

/// The file's new text after an install (`true`) or a structural remove. `None` is no file.
fn edit(format: Format, old: Option<&str>, install: bool) -> Result<Option<String>, String> {
    match (format, install) {
        (Format::Json { parent, stdio }, true) => jsonc::set(old, parent, &entry(stdio)).map(Some),
        (Format::Json { parent, .. }, false) => jsonc::remove(old, parent),
        (Format::Toml, true) => toml_set(old).map(Some),
        (Format::Toml, false) => toml_remove(old),
        (Format::Block, true) => Ok(Some(block_set(old, ""))),
        (Format::Block, false) => Ok(block_remove(old, "")),
        (Format::Rule, true) => Ok(Some(block_set(old, RULE_HEAD))),
        (Format::Rule, false) => Ok(block_remove(old, RULE_HEAD)),
    }
}

/// A remove that gives back the file's exact bytes when our install is the only change since:
/// the `.bak` from the install (if installing into it gives today's file), or no file at all (if
/// installing into nothing does). Otherwise the entry or block is taken out where it stands.
fn removed(format: Format, path: &Path, old: Option<&str>) -> Result<Option<String>, String> {
    let structural = edit(format, old, false)?;
    if structural.as_deref() == old {
        return Ok(structural);
    }
    if let Ok(bak) = std::fs::read_to_string(backup(path))
        && Some(bak.as_str()) != old
        && edit(format, Some(&bak), false).is_ok_and(|b| b.as_deref() == Some(&bak))
        && edit(format, Some(&bak), true).is_ok_and(|n| n.as_deref() == old)
    {
        return Ok(Some(bak));
    }
    if edit(format, None, true)?.as_deref() == old {
        return Ok(None);
    }
    Ok(structural)
}

fn find_block(t: &str) -> Option<(usize, usize)> {
    let s = t.find(START_PREFIX)?;
    let e = t[s..].find(END)? + s + END.len();
    Some((s, e))
}

/// Replaces what is between the markers, or appends the block after a newline.
fn block_set(old: Option<&str>, head: &str) -> String {
    let b = block();
    match old {
        Some(t) if !t.is_empty() => match find_block(t) {
            Some((s, e)) => format!("{}{b}{}", &t[..s], &t[e..]),
            None => format!("{t}\n{b}\n"),
        },
        _ => format!("{head}{b}\n"),
    }
}

/// Deletes the markers and the block, with the newline on each side that the install added.
/// `None` when nothing else is left (the file was ours).
fn block_remove(old: Option<&str>, head: &str) -> Option<String> {
    let t = old?;
    let Some((mut s, mut e)) = find_block(t) else {
        return Some(t.to_owned());
    };
    if t[e..].starts_with('\n') {
        e += 1;
    }
    if t[..s].ends_with('\n') {
        s -= 1;
    }
    let rest = format!("{}{}", &t[..s], &t[e..]);
    (!(rest.is_empty() || (!head.is_empty() && rest == head.trim_end()))).then_some(rest)
}

fn toml_ours(item: &toml_edit::Item) -> bool {
    item.get("command").and_then(|c| c.as_str()) == Some("cartograph")
        && item
            .get("args")
            .and_then(|a| a.as_array())
            .is_some_and(|a| a.len() == 1 && a.get(0).and_then(|v| v.as_str()) == Some("mcp"))
}

fn toml_set(old: Option<&str>) -> Result<String, String> {
    use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, value};
    let text = old.unwrap_or("");
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let empty = doc.as_table().is_empty();
    let servers = doc.entry("mcp_servers").or_insert_with(|| {
        let mut t = Table::new();
        t.set_implicit(true);
        Item::Table(t)
    });
    if servers.get(KEY).is_some_and(toml_ours) {
        return Ok(text.to_owned());
    }
    let args = Array::from_iter(["mcp"]);
    match servers {
        Item::Table(t) => {
            let mut ours = Table::new();
            ours["command"] = value("cartograph");
            ours["args"] = value(args);
            if !empty {
                ours.decor_mut().set_prefix("\n");
            }
            t.insert(KEY, Item::Table(ours));
        }
        Item::Value(toml_edit::Value::InlineTable(t)) => {
            let mut ours = InlineTable::new();
            ours.insert("command", "cartograph".into());
            ours.insert("args", args.into());
            t.insert(KEY, ours.into());
        }
        _ => return Err("`mcp_servers` isn't a table".into()),
    }
    Ok(doc.to_string())
}

fn toml_remove(old: Option<&str>) -> Result<Option<String>, String> {
    let Some(text) = old else { return Ok(None) };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let Some(servers) = doc
        .get_mut("mcp_servers")
        .and_then(|s| s.as_table_like_mut())
    else {
        return Ok(Some(text.to_owned()));
    };
    if servers.remove(KEY).is_none() {
        return Ok(Some(text.to_owned()));
    }
    if servers.is_empty() {
        doc.remove("mcp_servers");
    }
    let out = doc.to_string();
    Ok((!out.trim().is_empty()).then_some(out))
}

/// JSON with comments and trailing commas (VS Code's dialect), edited by byte spans so everything
/// outside the one member is kept as it was. No new dependency: a scanner that finds members.
mod jsonc {
    use serde_json::Value;

    pub struct Member {
        pub key: String,
        pub start: usize,
        pub value: (usize, usize),
        pub comma: Option<usize>,
    }

    pub struct Object {
        pub open: usize,
        pub close: usize,
        pub members: Vec<Member>,
    }

    struct P<'a> {
        s: &'a [u8],
        i: usize,
        depth: u32,
    }

    impl P<'_> {
        fn err<T>(&self, what: &str) -> Result<T, String> {
            Err(format!("{what} at byte {}", self.i))
        }

        fn peek(&self) -> Option<u8> {
            self.s.get(self.i).copied()
        }

        /// Whitespace and comments.
        fn ws(&mut self) -> Result<(), String> {
            loop {
                match (self.peek(), self.s.get(self.i + 1)) {
                    (Some(b' ' | b'\t' | b'\n' | b'\r'), _) => self.i += 1,
                    (Some(b'/'), Some(b'/')) => {
                        while self.peek().is_some_and(|c| c != b'\n') {
                            self.i += 1;
                        }
                    }
                    (Some(b'/'), Some(b'*')) => {
                        match self.s[self.i + 2..].windows(2).position(|w| w == b"*/") {
                            Some(p) => self.i += p + 4,
                            None => return self.err("an unclosed comment"),
                        }
                    }
                    _ => return Ok(()),
                }
            }
        }

        fn string(&mut self) -> Result<(), String> {
            self.i += 1;
            while let Some(c) = self.peek() {
                self.i += 1;
                match c {
                    b'\\' => self.i += 1,
                    b'"' => return Ok(()),
                    _ => {}
                }
            }
            self.err("an unclosed string")
        }

        fn nest(&mut self) -> Result<(), String> {
            self.depth += 1;
            if self.depth > 128 {
                return self.err("nesting deeper than 128");
            }
            Ok(())
        }

        fn value(&mut self) -> Result<(), String> {
            match self.peek() {
                Some(b'{') => self.object().map(drop),
                Some(b'[') => {
                    self.nest()?;
                    self.i += 1;
                    loop {
                        self.ws()?;
                        if self.peek() == Some(b']') {
                            break;
                        }
                        self.value()?;
                        self.ws()?;
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b']') => break,
                            _ => return self.err("expected , or ]"),
                        }
                    }
                    self.i += 1;
                    self.depth -= 1;
                    Ok(())
                }
                Some(b'"') => self.string(),
                _ => {
                    let start = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || b"+-.".contains(&c))
                    {
                        self.i += 1;
                    }
                    if start == self.i {
                        return self.err("expected a value");
                    }
                    Ok(())
                }
            }
        }

        fn object(&mut self) -> Result<Object, String> {
            self.nest()?;
            let open = self.i;
            self.i += 1;
            let mut members = Vec::new();
            loop {
                self.ws()?;
                match self.peek() {
                    Some(b'}') => break,
                    Some(b'"') => {}
                    _ => return self.err("expected a key or }"),
                }
                let start = self.i;
                self.string()?;
                let key = serde_json::from_slice::<String>(&self.s[start..self.i])
                    .map_err(|e| e.to_string())?;
                self.ws()?;
                if self.peek() != Some(b':') {
                    return self.err("expected :");
                }
                self.i += 1;
                self.ws()?;
                let vs = self.i;
                self.value()?;
                let ve = self.i;
                self.ws()?;
                let comma = (self.peek() == Some(b',')).then_some(self.i);
                members.push(Member {
                    key,
                    start,
                    value: (vs, ve),
                    comma,
                });
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(b'}') => break,
                    _ => return self.err("expected , or }"),
                }
            }
            let close = self.i;
            self.i += 1;
            self.depth -= 1;
            Ok(Object {
                open,
                close,
                members,
            })
        }
    }

    /// The top-level object; anything else is malformed.
    fn root(text: &str) -> Result<Object, String> {
        let mut p = P {
            s: text.as_bytes(),
            i: 0,
            depth: 0,
        };
        p.ws()?;
        if p.peek() != Some(b'{') {
            return p.err("expected an object");
        }
        let o = p.object()?;
        p.ws()?;
        if p.i != text.len() {
            return p.err("text after the object");
        }
        Ok(o)
    }

    /// The object at `at`, inside text [`root`] already checked.
    fn object_at(text: &str, at: usize) -> Result<Object, String> {
        P {
            s: text.as_bytes(),
            i: at,
            depth: 0,
        }
        .object()
    }

    fn line_indent(text: &str, pos: usize) -> &str {
        let ls = text[..pos].rfind('\n').map_or(0, |n| n + 1);
        let line = &text[ls..];
        &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
    }

    /// The indent of the object's members: the first one's, or one level in from the brace.
    fn member_indent(text: &str, o: &Object, unit: &str) -> String {
        if let Some(m) = o.members.first() {
            let ls = text[..m.start].rfind('\n').map_or(0, |n| n + 1);
            if ls > o.open && text[ls..m.start].trim().is_empty() {
                return text[ls..m.start].to_owned();
            }
        }
        format!("{}{unit}", line_indent(text, o.open))
    }

    fn quote(s: &str) -> String {
        Value::from(s).to_string()
    }

    /// The entry as text, in a fixed key order whatever serde_json's map keeps.
    fn render(v: &Value, indent: &str) -> String {
        let fields: Vec<String> = ["type", "command", "args"]
            .iter()
            .filter_map(|k| Some(format!("{indent}  {}: {}", quote(k), v.get(*k)?)))
            .collect();
        format!("{{\n{}\n{indent}}}", fields.join(",\n"))
    }

    /// Ours, if `command` and `args` (and `type`, when asked for) already say what ours would.
    fn same(text: &str, want: &Value) -> bool {
        let Ok(Value::Object(have)) = serde_json::from_str::<Value>(text) else {
            return false;
        };
        want.as_object()
            .is_some_and(|w| w.iter().all(|(k, v)| have.get(k) == Some(v)))
    }

    /// A member goes in first, so the remove can take back exactly what was added.
    fn insert_first(text: &str, o: &Object, indent: &str, member: &str) -> String {
        let at = o.open + 1;
        let tail = if o.members.is_empty() {
            if text[at..o.close].trim().is_empty() {
                let outer = line_indent(text, o.open);
                return format!(
                    "{}\n{indent}{member}\n{outer}{}",
                    &text[..at],
                    &text[o.close..]
                );
            }
            ""
        } else {
            ","
        };
        format!("{}\n{indent}{member}{tail}{}", &text[..at], &text[at..])
    }

    pub fn set(old: Option<&str>, parent: &str, v: &Value) -> Result<String, String> {
        let key = super::KEY;
        let Some(text) = old.filter(|t| !t.trim().is_empty()) else {
            return Ok(format!(
                "{{\n  {}: {{\n    {}: {}\n  }}\n}}\n",
                quote(parent),
                quote(key),
                render(v, "    ")
            ));
        };
        let r = root(text)?;
        let unit = match member_indent(text, &r, "  ") {
            u if u.is_empty() => "  ".to_owned(),
            u => u,
        };
        let Some(p) = r.members.iter().find(|m| m.key == parent) else {
            let ind = member_indent(text, &r, &unit);
            let inner = format!("{ind}{unit}");
            let member = format!(
                "{}: {{\n{inner}{}: {}\n{ind}}}",
                quote(parent),
                quote(key),
                render(v, &inner)
            );
            return Ok(insert_first(text, &r, &ind, &member));
        };
        if text.as_bytes()[p.value.0] != b'{' {
            return Err(format!("`{parent}` isn't an object"));
        }
        let o = object_at(text, p.value.0)?;
        match o.members.iter().find(|m| m.key == key) {
            Some(m) if same(&text[m.value.0..m.value.1], v) => Ok(text.to_owned()),
            Some(m) => Ok(format!(
                "{}{}{}",
                &text[..m.value.0],
                render(v, line_indent(text, m.start)),
                &text[m.value.1..]
            )),
            None => {
                let ind = member_indent(text, &o, &unit);
                Ok(insert_first(
                    text,
                    &o,
                    &ind,
                    &format!("{}: {}", quote(key), render(v, &ind)),
                ))
            }
        }
    }

    /// Takes the member out with its comma. The parent object stays, even when empty.
    pub fn remove(old: Option<&str>, parent: &str) -> Result<Option<String>, String> {
        let Some(text) = old else { return Ok(None) };
        let unchanged = Ok(Some(text.to_owned()));
        if text.trim().is_empty() {
            return unchanged;
        }
        let r = root(text)?;
        let Some(p) = r.members.iter().find(|m| m.key == parent) else {
            return unchanged;
        };
        if text.as_bytes()[p.value.0] != b'{' {
            return unchanged;
        }
        let o = object_at(text, p.value.0)?;
        let Some(i) = o.members.iter().position(|m| m.key == super::KEY) else {
            return unchanged;
        };
        let m = &o.members[i];
        let after_prev = if i == 0 {
            o.open + 1
        } else {
            o.members[i - 1].comma.map_or(o.open + 1, |c| c + 1)
        };
        let (from, to) = match (m.comma, i) {
            (Some(c), _) => (after_prev, c + 1),
            (None, 0)
                if text[m.value.1..o.close].trim().is_empty()
                    && text[o.open + 1..m.start].trim().is_empty() =>
            {
                (o.open + 1, o.close)
            }
            (None, 0) => (o.open + 1, m.value.1),
            (None, _) => (after_prev - 1, m.value.1),
        };
        Ok(Some(format!("{}{}", &text[..from], &text[to..])))
    }
}

// ---------------------------------------------------------------- the preview

/// A line diff with three lines of context around the one changed region. Common lines at both
/// ends are trimmed before the LCS, so a long `~/.claude.json` with one new entry stays cheap.
pub fn diff(file: &str, old: &str, new: &str) -> String {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let mut out = format!("--- {file}\n+++ {file}\n");
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..]
        .iter()
        .rev()
        .zip(b[pre..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    if am.is_empty() && bm.is_empty() {
        return out;
    }
    let from = pre.saturating_sub(3);
    out.push_str(&format!("@@ line {} @@\n", from + 1));
    a[from..pre]
        .iter()
        .for_each(|l| out.push_str(&format!(" {l}\n")));
    if am.len().saturating_mul(bm.len()) > 4_000_000 {
        am.iter().for_each(|l| out.push_str(&format!("-{l}\n")));
        bm.iter().for_each(|l| out.push_str(&format!("+{l}\n")));
    } else {
        let mut t = vec![vec![0u32; bm.len() + 1]; am.len() + 1];
        for i in (0..am.len()).rev() {
            for j in (0..bm.len()).rev() {
                t[i][j] = if am[i] == bm[j] {
                    t[i + 1][j + 1] + 1
                } else {
                    t[i + 1][j].max(t[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < am.len() || j < bm.len() {
            if i < am.len() && j < bm.len() && am[i] == bm[j] {
                out.push_str(&format!(" {}\n", am[i]));
                i += 1;
                j += 1;
            } else if i < am.len() && (j == bm.len() || t[i + 1][j] >= t[i][j + 1]) {
                out.push_str(&format!("-{}\n", am[i]));
                i += 1;
            } else {
                out.push_str(&format!("+{}\n", bm[j]));
                j += 1;
            }
        }
    }
    let end = a.len() - suf;
    a[end..(end + 3).min(a.len())]
        .iter()
        .for_each(|l| out.push_str(&format!(" {l}\n")));
    out
}

#[cfg(test)]
mod tests;
