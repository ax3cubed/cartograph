//! SRV-21's tests: a golden per writer (empty file, existing entries, comments, malformed), the
//! idempotence and round-trip property, and plan/apply in a temp home.

use super::*;
use proptest::prelude::*;

const CURSOR: Format = Format::Json {
    parent: "mcpServers",
    stdio: false,
};
const VSCODE: Format = Format::Json {
    parent: "servers",
    stdio: true,
};

fn ins(f: Format, old: Option<&str>) -> String {
    edit(f, old, true).unwrap().unwrap()
}

#[test]
fn json_golden_empty_file() {
    let want = r#"{
  "mcpServers": {
    "cartograph": {
      "command": "cartograph",
      "args": ["mcp"]
    }
  }
}
"#;
    assert_eq!(ins(CURSOR, None), want);
    assert_eq!(ins(CURSOR, Some("")), want);
    assert!(
        ins(VSCODE, None).contains("\"servers\"")
            && ins(VSCODE, None).contains("\"type\": \"stdio\"")
    );
}

#[test]
fn json_golden_existing_entries_and_comments() {
    let before = r#"{
    // my servers
    "servers": {
        "other": { "command": "x" }, // keep me
    },
    /* trailing */ "editor.fontSize": 14,
}
"#;
    let after = ins(VSCODE, Some(before));
    assert_eq!(
        after,
        r#"{
    // my servers
    "servers": {
        "cartograph": {
          "type": "stdio",
          "command": "cartograph",
          "args": ["mcp"]
        },
        "other": { "command": "x" }, // keep me
    },
    /* trailing */ "editor.fontSize": 14,
}
"#
    );
    assert_eq!(ins(VSCODE, Some(&after)), after, "idempotent");
    assert_eq!(
        edit(VSCODE, Some(&after), false).unwrap().unwrap(),
        before,
        "the structural remove gives back the bytes"
    );
}

#[test]
fn json_golden_missing_parent_and_empty_parent() {
    let before = "{\n  \"numStartups\": 3\n}\n";
    let after = ins(CURSOR, Some(before));
    assert_eq!(
        after,
        "{\n  \"mcpServers\": {\n    \"cartograph\": {\n      \"command\": \"cartograph\",\n      \"args\": [\"mcp\"]\n    }\n  },\n  \"numStartups\": 3\n}\n"
    );
    let before = "{\"mcpServers\": {}, \"a\": 1}";
    let after = ins(CURSOR, Some(before));
    assert!(after.starts_with("{\"mcpServers\": {\n  \"cartograph\": {"));
    assert_eq!(edit(CURSOR, Some(&after), false).unwrap().unwrap(), before);
}

#[test]
fn json_a_different_cartograph_entry_is_replaced_and_extra_keys_kept() {
    let before = r#"{"mcpServers": {"cartograph": {"command": "/old/cartograph", "args": []}}}"#;
    let after = ins(CURSOR, Some(before));
    assert!(after.contains("\"command\": \"cartograph\"") && !after.contains("/old"));
    let with_env =
        r#"{"mcpServers": {"cartograph": {"command": "cartograph", "args": ["mcp"], "env": {}}}}"#;
    assert_eq!(
        ins(CURSOR, Some(with_env)),
        with_env,
        "ours, plus the user's env"
    );
}

#[test]
fn json_malformed_is_refused() {
    for bad in [
        "{",
        "[1]",
        "{\"a\": }",
        "{} x",
        "{\"mcpServers\": 3}",
        "{/* open",
    ] {
        assert!(edit(CURSOR, Some(bad), true).is_err(), "{bad}");
    }
}

#[test]
fn toml_golden() {
    let want = "[mcp_servers.cartograph]\ncommand = \"cartograph\"\nargs = [\"mcp\"]\n";
    assert_eq!(ins(Format::Toml, None), want);
    let before = "# my codex\nmodel = \"o4\" # fast\n\n[mcp_servers.other]\ncommand = \"x\"\n";
    let after = ins(Format::Toml, Some(before));
    assert_eq!(after, format!("{before}\n{want}"));
    assert_eq!(ins(Format::Toml, Some(&after)), after);
    assert_eq!(
        edit(Format::Toml, Some(&after), false).unwrap().unwrap(),
        before
    );
    let plain = "model = \"o4\"\n";
    let after = ins(Format::Toml, Some(plain));
    assert_eq!(after, format!("{plain}\n{want}"));
    assert_eq!(
        edit(Format::Toml, Some(&after), false).unwrap().unwrap(),
        plain
    );
    assert!(edit(Format::Toml, Some("model = "), true).is_err());
    assert!(edit(Format::Toml, Some("mcp_servers = 3"), true).is_err());
}

#[test]
fn block_golden() {
    let b = block();
    assert!(b.starts_with("<!-- cartograph:start · managed by `cartograph agents install` -->\n## Codebase map (Cartograph)\n"));
    assert!(b.ends_with("instead of working around it.\n<!-- cartograph:end -->"));
    assert_eq!(ins(Format::Block, None), format!("{b}\n"));
    for before in ["# Notes\n", "# Notes", "# Notes\n\n"] {
        let after = ins(Format::Block, Some(before));
        assert_eq!(after, format!("{before}\n{b}\n"));
        assert_eq!(ins(Format::Block, Some(&after)), after);
        assert_eq!(
            edit(Format::Block, Some(&after), false).unwrap().unwrap(),
            before
        );
    }
    // An old block between the markers is replaced; what is around it stays.
    let old = "a\n<!-- cartograph:start old -->\nstale\n<!-- cartograph:end -->\nz\n";
    assert_eq!(ins(Format::Block, Some(old)), format!("a\n{b}\nz\n"));
    assert_eq!(
        edit(Format::Block, Some(&format!("{b}\n")), false).unwrap(),
        None
    );
    let rule = ins(Format::Rule, None);
    assert!(rule.starts_with(
        "---\ndescription: Cartograph codebase map\nalwaysApply: true\n---\n<!-- cartograph:start"
    ));
    assert_eq!(edit(Format::Rule, Some(&rule), false).unwrap(), None);
}

#[test]
fn the_diff_shows_the_change_with_context() {
    let old: String = (1..=20).map(|i| format!("l{i}\n")).collect();
    let new = old.replace("l10\n", "l10\nnew\n");
    let d = diff("f", &old, &new);
    assert_eq!(
        d,
        "--- f\n+++ f\n@@ line 8 @@\n l8\n l9\n l10\n+new\n l11\n l12\n l13\n"
    );
}

/// A file made of pieces: other members, comments, trailing commas.
fn jsonc_file() -> impl Strategy<Value = String> {
    let member = prop_oneof![
        Just("\"a\": 1".to_owned()),
        Just("\"b\": {\"c\": [1, 2,]}".to_owned()),
        Just("// note\n  \"d\": \"x\"".to_owned()),
        Just("\"mcpServers\": {}".to_owned()),
        Just("\"mcpServers\": {\n    \"other\": {\"command\": \"o\"}\n  }".to_owned()),
        Just("\"mcpServers\": { /* keep */ \"z\": {} , }".to_owned()),
    ];
    (
        prop::collection::vec(member, 0..4),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(ms, trailing, newline)| {
            let mut seen = false;
            let ms: Vec<String> = ms
                .into_iter()
                .filter(|m| !(m.contains("mcpServers") && std::mem::replace(&mut seen, true)))
                .collect();
            let mut s = format!("{{\n  {}", ms.join(",\n  "));
            if trailing && !ms.is_empty() {
                s.push(',');
            }
            s.push_str("\n}");
            if newline {
                s.push('\n');
            }
            s
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Install twice changes nothing the second time; remove after install gives the bytes back,
    /// through plan and apply on disk.
    #[test]
    fn install_is_idempotent_and_remove_restores(text in jsonc_file()) {
        let once = ins(CURSOR, Some(&text));
        prop_assert_eq!(ins(CURSOR, Some(&once)), once.clone());
        let parsed = json_strip(&once);
        prop_assert_eq!(&parsed["mcpServers"]["cartograph"]["command"], "cartograph");

        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".cursor")).unwrap();
        let file = home.path().join(".cursor/mcp.json");
        std::fs::write(&file, &text).unwrap();
        let place = Place { home: home.path().to_owned(), workspace: None, repos: vec![] };
        let run = |install: bool| {
            let c = plan(&place, Scope::default(), &[Agent::Cursor], install).unwrap();
            apply(&c).unwrap();
            c.len()
        };
        prop_assert_eq!(run(true), 1);
        prop_assert_eq!(run(true), 0);
        prop_assert_eq!(run(false), 1);
        prop_assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    }
}

/// The file as data, comments and trailing commas removed (a check that the output still parses).
fn json_strip(text: &str) -> Value {
    let mut out = String::new();
    let mut p = jsonc_strip_state(text);
    out.extend(p.by_ref());
    serde_json::from_str(&out).unwrap()
}

/// A tiny stripper for the test: drops `//` and `/* */` outside strings and commas before `}` `]`.
fn jsonc_strip_state(text: &str) -> std::vec::IntoIter<char> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut in_str) = (0, false);
    while i < c.len() {
        if in_str {
            out.push(c[i]);
            if c[i] == '\\' {
                out.push(c[i + 1]);
                i += 1;
            } else if c[i] == '"' {
                in_str = false;
            }
        } else if c[i] == '"' {
            in_str = true;
            out.push('"');
        } else if c[i] == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c[i] == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while !(c[i] == '*' && c[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if c[i] == '}' || c[i] == ']' {
            while out.last().is_some_and(|x: &char| x.is_whitespace()) {
                out.pop();
            }
            if out.last() == Some(&',') {
                out.pop();
            }
            out.push(c[i]);
        } else {
            out.push(c[i]);
        }
        i += 1;
    }
    out.into_iter()
}

fn place(home: &Path, ws: Option<&Path>, repos: &[&Path]) -> Place {
    Place {
        home: home.to_owned(),
        workspace: ws.map(Path::to_owned),
        repos: repos.iter().map(|r| r.to_path_buf()).collect(),
    }
}

#[test]
fn targets_by_scope() {
    let (h, w, r) = (Path::new("/h"), Path::new("/w"), Path::new("/w/r"));
    let p = place(h, Some(w), &[r]);
    let names = |a, s| -> Vec<String> {
        targets(a, &p, s, false)
            .into_iter()
            .map(|(p, _)| p.display().to_string())
            .collect()
    };
    assert_eq!(
        names(Agent::ClaudeCode, Scope::default()),
        ["/h/.claude.json", "/w/CLAUDE.local.md"]
    );
    assert_eq!(
        names(
            Agent::ClaudeCode,
            Scope {
                shared: true,
                ..Default::default()
            }
        ),
        ["/h/.claude.json", "/w/CLAUDE.md"]
    );
    assert_eq!(
        names(
            Agent::Cursor,
            Scope {
                project: true,
                ..Default::default()
            }
        ),
        ["/w/.cursor/mcp.json", "/w/.cursor/rules/cartograph.mdc"]
    );
    assert_eq!(
        names(
            Agent::Codex,
            Scope {
                per_repo: true,
                ..Default::default()
            }
        ),
        ["/w/r/.codex/config.toml", "/w/r/AGENTS.md"]
    );
    assert_eq!(names(Agent::VsCode, Scope::default()).len(), 1);
    assert!(
        targets(Agent::ClaudeCode, &p, Scope::default(), true).is_empty(),
        "the plugin covers it"
    );
}

#[test]
fn end_to_end_in_a_temp_home() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let ws = h.join("work");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(h.join(".codex")).unwrap();
    let codex = h.join(".codex/config.toml");
    std::fs::write(&codex, "# mine\nmodel = \"o4\"\n").unwrap();
    std::fs::write(h.join(".claude.json"), "{\"numStartups\": 1}").unwrap();
    let p = place(h, Some(&ws), &[]);
    let all = Agent::ALL;
    let before: Vec<(PathBuf, Option<String>)> = [
        codex.clone(),
        h.join(".claude.json"),
        ws.join("CLAUDE.local.md"),
        h.join(".cursor/mcp.json"),
    ]
    .into_iter()
    .map(|f| (f.clone(), std::fs::read_to_string(&f).ok()))
    .collect();

    let changes = plan(&p, Scope::default(), &all, true).unwrap();
    assert!(changes.iter().all(|c| !c.diff().is_empty()));
    apply(&changes).unwrap();
    assert!(
        plan(&p, Scope::default(), &all, true).unwrap().is_empty(),
        "second install"
    );
    let st = status(h, Some(&ws));
    assert!(st.iter().all(|s| s.installed), "{st:?}");
    assert_eq!(
        std::fs::read_to_string(h.join(".claude.json.bak")).unwrap(),
        "{\"numStartups\": 1}"
    );
    // The plugin present: nothing more for Claude Code.
    std::fs::create_dir_all(h.join(".claude/plugins")).unwrap();
    std::fs::write(
        h.join(".claude/plugins/installed_plugins.json"),
        r#"{"version":2,"plugins":{"cartograph@cartograph":[]}}"#,
    )
    .unwrap();
    assert!(plugin_installed(h));

    apply(&plan(&p, Scope::default(), &all, false).unwrap()).unwrap();
    for (f, b) in before {
        assert_eq!(std::fs::read_to_string(&f).ok(), b, "{}", f.display());
    }
    assert!(
        status(h, Some(&ws))
            .iter()
            .all(|s| !s.installed || s.id == "claude-code")
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_home_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let target = elsewhere.path().join("mcp.json");
    std::fs::write(&target, "{}").unwrap();
    std::fs::create_dir(home.path().join(".cursor")).unwrap();
    std::os::unix::fs::symlink(&target, home.path().join(".cursor/mcp.json")).unwrap();
    let p = place(home.path(), None, &[]);
    assert!(matches!(
        plan(&p, Scope::default(), &[Agent::Cursor], true),
        Err(AgentError::Outside { .. })
    ));
    // Inside home, the link is followed and stays a link.
    let inner = home.path().join("real.json");
    std::fs::write(&inner, "{}").unwrap();
    std::fs::remove_file(home.path().join(".cursor/mcp.json")).unwrap();
    std::os::unix::fs::symlink(&inner, home.path().join(".cursor/mcp.json")).unwrap();
    apply(&plan(&p, Scope::default(), &[Agent::Cursor], true).unwrap()).unwrap();
    assert!(
        std::fs::symlink_metadata(home.path().join(".cursor/mcp.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::read_to_string(&inner)
            .unwrap()
            .contains("cartograph")
    );
}

#[test]
fn a_malformed_file_is_left_alone() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".cursor")).unwrap();
    std::fs::write(home.path().join(".cursor/mcp.json"), "{ nope").unwrap();
    let p = place(home.path(), None, &[]);
    let e = plan(&p, Scope::default(), &[Agent::Cursor], true).unwrap_err();
    assert!(matches!(e, AgentError::Malformed { .. }), "{e}");
    assert!(
        plan(
            &p,
            Scope {
                project: true,
                ..Default::default()
            },
            &[Agent::Cursor],
            true
        )
        .is_err()
    );
}

#[test]
fn the_server_calls() {
    let home = tempfile::tempdir().unwrap();
    let req = AgentInstall {
        agent: "cursor".into(),
        shared: false,
    };
    let d = preview(home.path(), None, &req).unwrap();
    assert_eq!(d.len(), 1);
    assert!(!d[0].applied && d[0].diff.contains("+    \"cartograph\": {"));
    assert!(
        !home.path().join(".cursor/mcp.json").exists(),
        "a preview writes nothing"
    );
    assert!(super::install(home.path(), None, &req).unwrap()[0].applied);
    assert!(preview(home.path(), None, &req).unwrap().is_empty());
    assert!(matches!(
        preview(
            home.path(),
            None,
            &AgentInstall {
                agent: "vim".into(),
                shared: false
            }
        ),
        Err(AgentError::UnknownAgent(_))
    ));
}
