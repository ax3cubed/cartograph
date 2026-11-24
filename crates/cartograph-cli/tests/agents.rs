//! SRV-21 end to end: `cartograph agents` in a temp HOME. Install twice changes nothing the second
//! time, remove gives back each file byte for byte (apart from the `.bak`), and a mcp.json with
//! comments keeps them.
#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output, Stdio};

fn cartograph(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("CARTOGRAPH_HOME", home.join("data"))
        // No agent binaries: only the folders below decide what is found.
        .env("PATH", home.join("no-bin"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn ok(o: &Output) -> String {
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout.clone()).unwrap()
}

const CURSOR: &str = "{
  // servers I use
  \"mcpServers\": {
    \"other\": { \"command\": \"x\" }, /* keep */
  },
}
";
const CODEX: &str = "# my settings\nmodel = \"o4\"\n";

#[test]
fn install_twice_then_remove_gives_the_bytes_back() {
    let d = tempfile::tempdir().unwrap();
    let home = &std::fs::canonicalize(d.path()).unwrap();
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(home.join(".cursor/mcp.json"), CURSOR).unwrap();
    std::fs::write(home.join(".codex/config.toml"), CODEX).unwrap();

    // No terminal and no --yes: refused, nothing written.
    let o = cartograph(home, &["agents", "install"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--yes"));
    assert_eq!(
        std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap(),
        CURSOR
    );

    let first = ok(&cartograph(home, &["agents", "install", "--yes"]));
    assert!(
        first.contains("+    \"cartograph\": {"),
        "the diff is shown: {first}"
    );
    assert!(first.contains("updated ") && first.contains("(Cursor)") && first.contains("(Codex)"));
    let cursor = std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap();
    assert!(cursor.contains("// servers I use") && cursor.contains("/* keep */"));
    assert!(
        std::fs::read_to_string(home.join(".codex/config.toml"))
            .unwrap()
            .contains("[mcp_servers.cartograph]\ncommand = \"cartograph\"\nargs = [\"mcp\"]\n")
    );
    assert!(
        !home.join(".claude.json").exists(),
        "Claude Code isn't on this machine"
    );

    assert_eq!(
        ok(&cartograph(home, &["agents", "install", "--yes"])).trim(),
        "nothing to change"
    );
    let st: serde_json::Value =
        serde_json::from_str(&ok(&cartograph(home, &["agents", "status", "--json"]))).unwrap();
    let row = |id: &str| {
        st["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(row("cursor")["installed"], true);
    assert_eq!(row("codex")["installed"], true);
    assert_eq!(row("vscode")["detected"], false);

    ok(&cartograph(home, &["agents", "remove", "--yes"]));
    assert_eq!(
        std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap(),
        CURSOR
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".codex/config.toml")).unwrap(),
        CODEX
    );
    assert!(home.join(".cursor/mcp.json.bak").exists());
}

#[test]
fn a_malformed_config_is_refused_and_left_alone() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    std::fs::write(home.join(".cursor/mcp.json"), "{ \"mcpServers\": ").unwrap();
    let o = cartograph(home, &["agents", "install", "--yes"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("fix the file by hand"));
    assert_eq!(
        std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap(),
        "{ \"mcpServers\": "
    );
}

#[test]
fn project_needs_a_workspace() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".cursor")).unwrap();
    let o = cartograph(d.path(), &["agents", "install", "--project", "--yes"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("cartograph init"));
}
