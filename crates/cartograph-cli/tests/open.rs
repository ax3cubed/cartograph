//! `cartograph serve` and `cartograph open` (SRV-05, SRV-06): the binary serves the app and the API
//! on loopback, and a second `open` finds the running server instead of starting another.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

fn cartograph(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cartograph"));
    c.args(args)
        .env("CARTOGRAPH_HOME", home)
        .stdin(Stdio::null());
    c
}

struct Killed(Child);
impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn http(port: u16, head: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(
        format!("{head}\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).unwrap();
    buf
}

#[test]
fn serve_prints_the_address_and_open_reuses_the_running_server() {
    let d = tempfile::tempdir().unwrap();
    let mut child = Killed(
        cartograph(d.path(), &["serve", "--json"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    let url = v["url"].as_str().unwrap().to_owned();
    let port = v["port"].as_u64().unwrap() as u16;
    let token = url.split_once("/#k=").unwrap().1.to_owned();
    assert_eq!(url, format!("http://127.0.0.1:{port}/#k={token}"));
    assert_eq!(token.len(), 43);

    let api = http(
        port,
        &format!("GET /api/version HTTP/1.1\r\nAuthorization: Bearer {token}"),
    );
    assert!(api.starts_with("HTTP/1.1 200"), "{api}");
    let page = http(port, "GET / HTTP/1.1");
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(
        page.to_ascii_lowercase()
            .contains("content-security-policy:")
    );
    let traversal = http(port, "GET /..%2f..%2fetc/passwd HTTP/1.1");
    assert!(traversal.starts_with("HTTP/1.1 404"), "{traversal}");

    // A second open prints the same address and returns at once.
    let o = cartograph(d.path(), &["open", "--json"]).output().unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let again: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(again["url"], url.as_str());
}

#[test]
fn a_lan_host_is_refused_with_exit_1() {
    let d = tempfile::tempdir().unwrap();
    let o = cartograph(d.path(), &["serve", "--host", "0.0.0.0"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("not a loopback address") && err.contains("this machine only"),
        "{err}"
    );
    assert!(!d.path().join("server.json").exists());
}
