use std::path::Path;
use std::time::Duration;

use super::*;
use crate::schema::AttrValue;

/// The adapter's behaviour is the file's content, so one script covers every fault.
const SCRIPT: &str = r#"
import sys, json, time, os, socket
def fact(r, **kw):
    d = {"type": "fact", "id": r["id"], "blob": r["blob"], "path": r["path"], "kind": "route",
         "span": [1, 0, 1, 3], "attrs": {"method": "GET", "path": "/x"},
         "evidence": {"type": "literal", "text": "GET /x"}}
    d.update(kw)
    print(json.dumps(d), flush=True)
def probe(f):
    try:
        f()
        return "ok"
    except Exception as e:
        return "denied"
for line in sys.stdin:
    r = json.loads(line)
    m = r["content"]
    if m == "ok": fact(r)
    elif m == "lie": fact(r, path="other.py")
    elif m == "lieblob": fact(r, blob="00")
    elif m == "hang": time.sleep(60)
    elif m == "flood": sys.stdout.write("x" * 100000); sys.stdout.flush()
    elif m == "midline": sys.stdout.write('{"type":"do'); sys.stdout.flush(); sys.exit(0)
    elif m == "badkind": fact(r, kind="nope")
    elif m == "span": fact(r, span=[9, 0, 9, 3])
    elif m == "secret": fact(r, evidence={"type": "literal", "text": "key AKIAIOSFODNN7EXAMPLE " + "y" * 500})
    elif m == "many":
        for _ in range(10001): fact(r)
    elif m.startswith("{"):
        p = json.loads(m)
        res = {
            "own": probe(lambda: open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "adapter.toml")).read()),
            "write_own": probe(lambda: open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "w.txt"), "w").write("x")),
            "write_other": probe(lambda: open(p["write"], "w").write("x")),
            "read_other": probe(lambda: open(p["read"]).read()),
            "net": probe(lambda: socket.create_connection(("127.0.0.1", p["port"]), 2)),
        }
        fact(r, attrs=res)
    print(json.dumps({"type": "done", "id": r["id"]}), flush=True)
"#;

fn setup(root: &Path) -> Option<Declared> {
    let dir = root.join("ad");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("adapter.toml"),
        "name = \"py\"\nversion = \"1\"\ncommand = \"python3\"\nargs = [\"adapter.py\"]\nfiles = [\"**/*.py\"]\n",
    )
    .unwrap();
    std::fs::write(dir.join("adapter.py"), SCRIPT).unwrap();
    match Declared::load(root, "ad") {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!("skipped: no python3 to run the sample adapter: {e}");
            None
        }
    }
}

fn set(d: Declared) -> SubSet {
    SubSet {
        wall: Some(Duration::from_secs(3)),
        // The fault tests don't need the sandbox; the sandbox tests insist on one.
        unsandboxed: Sandbox::detect() == Sandbox::None,
        ..SubSet::new(vec![Ok(d)], |_| true)
    }
}

fn run(set: &SubSet, cache: &mut FactCache, content: &str) -> (Vec<BlobFact>, Vec<String>) {
    let mut s = set.session();
    let got = s.extract(cache, b"\x01\x02", "a.py", || {
        Some(content.as_bytes().to_vec())
    });
    let facts = got.into_iter().flat_map(|a| a.facts).collect();
    (facts, s.notes)
}

fn cache(dir: &Path) -> FactCache {
    FactCache::open(&dir.join("facts.db")).unwrap()
}

#[test]
fn a_sample_adapter_makes_facts_and_runs_once_per_blob() {
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    let (set, mut c) = (set(d), cache(t.path()));
    let (facts, notes) = run(&set, &mut c, "ok");
    assert_eq!((facts.len(), notes.len()), (1, 0), "{notes:?}");
    assert_eq!(facts[0].kind, crate::schema::FactKind::Route);
    // The cache answers: the bytes are never read.
    let mut s = set.session();
    let got = s.extract(&mut c, b"\x01\x02", "a.py", || panic!("read on a hit"));
    assert_eq!(got[0].facts, facts);
    assert!(s.procs.is_empty());
}

#[test]
fn a_line_that_names_another_file_is_rejected() {
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    let (set, mut c) = (set(d), cache(t.path()));
    for m in ["lie", "lieblob"] {
        let (facts, notes) = run(&set, &mut c, m);
        assert!(facts.is_empty(), "{m}");
        assert!(
            notes.iter().any(|n| n.contains("names another")),
            "{m}: {notes:?}"
        );
        // Nothing was cached for the blob.
        assert!(c.get_interface(b"\x01\x02", "subprocess-py", "1").is_none());
    }
}

#[test]
fn faults_stop_the_adapter_and_cache_nothing() {
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    let set = set(d);
    for (m, why) in [
        ("hang", "timed out"),
        ("flood", "64 KiB"),
        ("midline", "middle of a line"),
        ("badkind", "invalid fact"),
        ("span", "outside the file"),
        ("many", "10000 facts"),
    ] {
        let mut c = cache(t.path());
        let (facts, notes) = run(&set, &mut c, m);
        assert!(facts.is_empty(), "{m}");
        assert!(notes.iter().any(|n| n.contains(why)), "{m}: {notes:?}");
        assert!(
            c.get_interface(b"\x01\x02", "subprocess-py", "1").is_none(),
            "{m}"
        );
    }
}

#[test]
fn evidence_is_capped_and_masked() {
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    let (set, mut c) = (set(d), cache(t.path()));
    let (facts, notes) = run(&set, &mut c, "secret");
    assert!(notes.is_empty(), "{notes:?}");
    let text = facts[0].evidence.text.as_str();
    assert!(
        !text.contains("AKIAIOSFODNN7EXAMPLE") && text.chars().count() <= 200,
        "{text}"
    );
}

#[test]
fn an_adapter_stopped_once_is_not_started_again_in_the_run() {
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    let (set, mut c) = (set(d), cache(t.path()));
    let mut s = set.session();
    s.extract(&mut c, b"\x01", "a.py", || Some(b"lie".to_vec()));
    let got = s.extract(&mut c, b"\x02", "b.py", || Some(b"ok".to_vec()));
    assert!(got.is_empty());
    assert_eq!(s.notes.len(), 1);
}

#[test]
fn nothing_runs_unapproved_and_the_reason_is_given() {
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    let set = SubSet::new(vec![Ok(d)], |_| false);
    assert!(set.is_empty());
    assert!(
        set.skipped[0].contains("not approved") && set.skipped[0].contains("adapters allow py")
    );
    assert_eq!(set.version(), "");
}

#[test]
fn a_changed_folder_changes_the_hash() {
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    std::fs::write(t.path().join("ad/adapter.py"), "print(1)").unwrap();
    assert_ne!(Declared::load(t.path(), "ad").unwrap().hash, d.hash);
}

#[test]
fn a_manifest_cannot_pick_a_folder_outside_its_own() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let outside = t.path().join("evil");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("adapter.toml"), "x").unwrap();
    for f in ["../evil", "/tmp", "", ".", "a/../../evil", "a\\b"] {
        assert!(Declared::load(&root, f).is_err(), "{f}");
    }
    // A symlink out of the repository is no way in.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert!(Declared::load(&root, "link").is_err());
    }
    // Nor is a symlink inside the folder.
    let Some(_) = setup(&root) else { return };
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hosts", root.join("ad/x")).unwrap();
        assert!(Declared::load(&root, "ad").is_err());
    }
}

#[test]
fn a_command_cannot_leave_the_folder() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("ad");
    std::fs::create_dir_all(&dir).unwrap();
    for c in ["../x", "/bin/sh", "a/../../x"] {
        std::fs::write(
            dir.join("adapter.toml"),
            format!("name = \"py\"\nversion = \"1\"\ncommand = \"{c}\"\nfiles = [\"*\"]\n"),
        )
        .unwrap();
        assert!(Declared::load(t.path(), "ad").is_err(), "{c}");
    }
}

#[test]
fn without_a_sandbox_the_adapter_is_refused_unless_asked() {
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    if Sandbox::detect() != Sandbox::None {
        eprintln!("skipped: this system has a sandbox, so the refusal can't be seen");
        return;
    }
    let mut c = cache(t.path());
    let mut set = set(d);
    set.unsandboxed = false;
    let (facts, notes) = run(&set, &mut c, "ok");
    assert!(
        facts.is_empty() && notes[0].contains("--unsandboxed-adapters"),
        "{notes:?}"
    );
}

#[test]
fn the_sandbox_denies_network_writes_and_reads_outside_the_folder() {
    if Sandbox::detect() == Sandbox::None {
        eprintln!("skipped: no sandbox-exec or bwrap on this system");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let Some(d) = setup(t.path()) else { return };
    let mut set = set(d);
    set.unsandboxed = false;
    let secret = t.path().join("secret.txt");
    std::fs::write(&secret, "s").unwrap();
    let other = t.path().join("written.txt");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let content = serde_json::json!({ "write": other, "read": secret, "port": port }).to_string();
    let mut c = cache(t.path());
    let (facts, notes) = run(&set, &mut c, &content);
    assert!(notes.is_empty(), "{notes:?}");
    let get = |k: &str| match facts[0].attrs.get(k) {
        Some(AttrValue::Str(s)) => s.as_str().to_owned(),
        _ => String::new(),
    };
    assert_eq!(get("own"), "ok", "the adapter reads its own folder");
    for k in ["write_own", "write_other", "read_other", "net"] {
        assert_eq!(get(k), "denied", "{k}");
    }
    assert!(!other.exists() && !t.path().join("ad/w.txt").exists());
    assert!(listener.accept().is_err(), "the adapter connected");
}

proptest::proptest! {
    /// The reader of an adapter's lines (the fuzz target of XS-24): any bytes give a fact, a done or
    /// an error, never a panic.
    #[test]
    fn a_line_of_any_bytes_never_panics(line in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)) {
        let _ = parse_line(&line, 1, "ab", "a.py", 3);
    }

    #[test]
    fn json_of_any_shape_never_panics(
        ty in "(fact|done|x)", id in 0u64..3, path in "[a-z.]{0,4}", kind in "[a-z.]{0,8}",
        span in proptest::collection::vec(0u32..6, 4),
    ) {
        let line = serde_json::json!({
            "type": ty, "id": id, "blob": "ab", "path": path, "kind": kind, "span": span,
            "evidence": {"type": "literal", "text": "t"}
        })
        .to_string();
        let _ = parse_line(line.as_bytes(), 1, "ab", "a.py", 3);
    }

    #[test]
    fn the_line_reader_bounds_every_line(data in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..200_000)) {
        let (tx, rx) = sync_channel(1_000_000);
        read_lines(&data[..], tx);
        for l in rx.into_iter().flatten() {
            proptest::prop_assert!(l.len() <= MAX_LINE);
        }
    }
}
