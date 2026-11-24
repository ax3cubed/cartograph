//! SRV-15 and SRV-17 end to end: `cartograph mcp` over stdio speaks MCP (initialize, tools/list,
//! tools/call, resources/read), stdout carries frames only, and `cartograph tool` prints the same
//! JSON the server returns for the same call.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn cartograph(home: &Path, dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cartograph"));
    c.env("CARTOGRAPH_HOME", home).current_dir(dir);
    c
}

#[test]
fn the_stdio_server_lists_eight_tools_and_tool_prints_what_it_returns() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let repo = cartograph_fixtures::northwind();
    let ok = cartograph(&home, &repo)
        .args(["index", "-q"])
        .arg(&repo)
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );

    let mut child = cartograph(&home, &repo)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut send = |v: Value| writeln!(stdin, "{v}").unwrap();
    let mut recv = |id: u64| -> Value {
        loop {
            let line = lines.next().expect("a frame").unwrap();
            let v: Value = serde_json::from_str(&line).expect("stdout holds JSON-RPC frames only");
            if v["id"] == id {
                return v;
            }
        }
    };
    send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "test", "version": "0"}}}),
    );
    let init = recv(1);
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.len() < 4096);
    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let list = recv(2);
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 8, "{names:?}");
    let args = json!({"query": "invoice"});
    send(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "find", "arguments": args}}));
    let call = recv(3);
    assert_eq!(call["result"]["isError"], false, "{call}");
    let structured = call["result"]["structuredContent"].clone();
    assert!(
        call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("_Index: ")
    );
    send(
        json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read",
        "params": {"uri": "cartograph://status"}}),
    );
    let res = recv(4);
    assert!(
        res["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Workspace")
    );
    send(json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": {"name": "find", "arguments": {"query": "x", "nope": true}}}));
    assert_eq!(recv(5)["result"]["isError"], true);
    drop(stdin);
    child.wait().unwrap();

    let tool = cartograph(&home, &repo)
        .args(["tool", "find", &args.to_string()])
        .output()
        .unwrap();
    assert!(
        tool.status.success(),
        "{}",
        String::from_utf8_lossy(&tool.stderr)
    );
    let printed: Value = serde_json::from_slice(&tool.stdout).unwrap();
    assert_eq!(
        printed, structured,
        "cartograph tool prints what the server returns"
    );

    // Arguments on stdin, Markdown out; and the registry from mcp --list.
    let mut md = cartograph(&home, &repo)
        .args(["tool", "describe", "--format", "md"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    md.stdin
        .take()
        .unwrap()
        .write_all(br#"{"node":"billing-service"}"#)
        .unwrap();
    let out = md.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("_Index: "));
    let listed = cartograph(&home, &repo)
        .args(["--json", "mcp", "--list"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(v["tools"].as_array().unwrap().len(), 8);
}
