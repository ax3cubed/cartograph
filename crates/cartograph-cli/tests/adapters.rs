//! Subprocess adapters (XS-24), end to end: a sample Python adapter's facts become HTTP links once
//! the user allows it, a fresh machine with the same manifest indexes without running it and says
//! why, and a changed adapter asks again.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use cartograph_discovery::manifest::Service;
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::sync::{SyncOutcome, SyncSummary};

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "# route GET /orders/:id\n";
const CLIENT: &str = "# call GET http://orders:8080/orders/1\n";
const TOML: &str = "name = \"py\"\nversion = \"1\"\ncommand = \"python3\"\nargs = [\"adapter.py\"]\nfiles = [\"orders/**/*.py\", \"web/**/*.py\"]\n";
/// Reads `# route METHOD PATH` and `# call METHOD URL` lines.
const ADAPTER: &str = r##"
import sys, json
for line in sys.stdin:
    r = json.loads(line)
    for n, text in enumerate(r["content"].split("\n"), 1):
        w = text.split()
        if len(w) == 4 and w[0] == "#" and w[1] in ("route", "call"):
            route = w[1] == "route"
            attrs = {"method": w[2], "path" if route else "url": w[3]}
            print(json.dumps({"type": "fact", "id": r["id"], "blob": r["blob"], "path": r["path"],
                "kind": "route" if route else "http.call", "span": [n, 0, n, len(text)],
                "attrs": attrs, "evidence": {"type": "literal", "text": text}}), flush=True)
    print(json.dumps({"type": "done", "id": r["id"]}), flush=True)
"##;

fn cartograph(home: &Path, dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .args(args)
        .arg("--dir")
        .arg(dir)
        .env("CARTOGRAPH_HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Registers the repo in the data folder `data`.
fn register(f: &Fixture, data: &Path) -> String {
    let mut plan = cartograph_discovery::setup::plan(&f.dir, None).unwrap();
    let repo = plan.manifest.repos[0].name.clone();
    plan.manifest.services = ["orders", "web"]
        .map(|n| Service {
            name: n.into(),
            repo: repo.clone(),
            path: Some(n.into()),
            tags: vec![],
        })
        .into();
    plan.manifest.adapters = vec!["tools/py".into()];
    cartograph_discovery::setup::apply(&plan, data)
        .unwrap()
        .workspace_id
}

fn sync(data: &Path, id: &str) -> (SyncSummary, PathBuf) {
    let ws = cartograph_server::workspace(data, id).unwrap();
    let mut cache = FactCache::open(&data.join("cache/facts.db")).unwrap();
    let SyncOutcome::Synced(s) =
        cartograph_index::sync::sync(&ws, &Default::default(), &mut cache).unwrap()
    else {
        panic!("another sync is running");
    };
    (s, ws.index_path())
}

fn http_links(index: &Path) -> i64 {
    let ix = cartograph_store::Index::open_read(index).unwrap();
    ix.conn
        .query_row("SELECT count(*) FROM link WHERE kind = 'http'", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn a_sample_adapter_makes_links_after_allow_and_asks_again_when_changed() {
    if Command::new("python3").arg("-V").output().is_err() {
        eprintln!("skipped: no python3");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let mut f = Fixture::init(&root.join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/server.py", SERVER);
    f.write("web/client.py", CLIENT);
    f.write("tools/py/adapter.toml", TOML);
    f.write("tools/py/adapter.py", ADAPTER);
    f.commit("two services and an adapter");

    // This machine has not approved it: the sync says so, runs nothing, and finds no link.
    let data = root.join("data");
    let id = register(&f, &data);
    let (_, index) = sync(&data, &id);
    assert_eq!(http_links(&index), 0);
    let o = cartograph(&data, &f.dir, &["sync"]);
    let said = text(&o);
    assert!(
        said.contains("adapter py not run: not approved on this machine")
            && said.contains("cartograph adapters allow py"),
        "{said}"
    );
    // Syncing never approves.
    let registry = std::fs::read_to_string(cartograph_core::paths::registry_file(&data)).unwrap();
    assert!(!registry.contains("adapter"), "{registry}");

    // `list` shows it, `allow` approves it, and the next sync turns its facts into a link.
    assert!(text(&cartograph(&data, &f.dir, &["adapters", "list"])).contains("not approved"));
    let o = cartograph(&data, &f.dir, &["adapters", "allow", "py"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let (s, index) = sync(&data, &id);
    assert!(s.adapter_notes.is_empty(), "{:?}", s.adapter_notes);
    assert_eq!(http_links(&index), 1, "the adapter's route and call link");

    // The same manifest on another machine (another data folder): nothing runs, and it says why.
    let other = root.join("other");
    let id2 = register(&f, &other);
    let (_, index) = sync(&other, &id2);
    assert_eq!(http_links(&index), 0);
    let said = text(&cartograph(&other, &f.dir, &["sync"]));
    assert!(said.contains("not approved on this machine"), "{said}");

    // A changed adapter is asked about again, and `deny` withdraws the approval.
    f.write("tools/py/adapter.py", &format!("{ADAPTER}\n# changed\n"));
    f.commit("change the adapter");
    let said = text(&cartograph(&data, &f.dir, &["adapters", "list"]));
    assert!(said.contains("changed"), "{said}");
    let (s, _) = sync(&data, &id);
    assert!(
        cartograph_server::workspace(&data, &id)
            .unwrap()
            .subs
            .skipped
            .len()
            == 1,
        "{s:?}"
    );
    assert_eq!(
        cartograph(&data, &f.dir, &["adapters", "deny", "py"])
            .status
            .code(),
        Some(0)
    );
}
