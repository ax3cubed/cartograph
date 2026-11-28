//! `cartograph check` (XS-12): exit 1 naming the entry and its line when a correction's handler is
//! deleted; `stale: warn` makes it a warning, `stale: off` says nothing.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use cartograph_discovery::manifest::Service;
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;

const COMPOSE: &str = "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n";
const SERVER: &str = "import express from \"express\";\nconst app = express();\n\napp.get(\"/orders/:id\", (req, res) => res.json({}));\n";
const API: &str =
    "export function load(id: string) {\n  return fetch(\"http://orders:8080/orders/\" + id);\n}\n";
const ENTRY: &str = "corrections:
  - action: set
    key: {kind: http, from: 'web:web/src/api.ts', to: 'orders:orders/src/server.ts', op: 'GET /orders/{id}'}
    confidence: exact
    reason: the contract says so
";

fn cartograph(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cartograph"))
        .args(args)
        .env("CARTOGRAPH_HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

struct Ws {
    data: std::path::PathBuf,
    id: String,
    f: Fixture,
    cache: FactCache,
}

impl Ws {
    fn sync(&mut self) {
        let ws = cartograph_server::workspace(&self.data, &self.id).unwrap();
        cartograph_index::sync::sync(&ws, &Default::default(), &mut self.cache).unwrap();
    }
    fn check(&self, extra: &[&str]) -> Output {
        let mut args = vec!["check"];
        args.extend(extra);
        args.push(self.f.dir.to_str().unwrap());
        cartograph(&self.data, &args)
    }
}

fn workspace(root: &Path) -> Ws {
    let data = root.join("data");
    let mut f = Fixture::init(&root.join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/src/server.ts", SERVER);
    f.write("web/src/api.ts", API);
    f.commit("two services");
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
    let applied = cartograph_discovery::setup::apply(&plan, &data).unwrap();
    let cache = FactCache::open(&data.join("cache/facts.db")).unwrap();
    Ws {
        data,
        id: applied.workspace_id,
        f,
        cache,
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn a_deleted_handler_fails_check_unless_stale_is_warn_or_off() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let mut w = workspace(&root);

    // No file, nothing to fail.
    w.sync();
    let o = w.check(&[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));

    w.f.write("cartograph.overrides.yaml", &format!("version: 1\n{ENTRY}"));
    w.f.commit("a correction");
    w.sync();
    let o = w.check(&["--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let j: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(j["schema"], 1);
    assert_eq!(j["rules"], serde_json::json!([]));
    assert_eq!(j["corrections"][0]["status"], "bound");
    assert_eq!(j["corrections"][0]["line"], 3);
    assert_eq!(
        j["corrections"][0]["untrusted_text"],
        "the contract says so"
    );

    // The handler goes: the entry matches nothing, and check says which and where.
    w.f.git(&["rm", "-q", "orders/src/server.ts"]);
    w.f.commit("drop the handler");
    w.sync();
    let o = w.check(&[]);
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(
        err.contains("cartograph.overrides.yaml:3 stale set http: web:web/src/api.ts -> orders:orders/src/server.ts GET /orders/{id}"),
        "{err}"
    );

    // `stale: warn`: the same line, exit 0.
    w.f.write(
        "cartograph.overrides.yaml",
        &format!("version: 1\nstale: warn\n{ENTRY}"),
    );
    w.sync();
    let o = w.check(&[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert!(text(&o.stderr).contains("warning: cartograph.overrides.yaml:4 stale"));

    // `stale: off`: silent.
    w.f.write(
        "cartograph.overrides.yaml",
        &format!("version: 1\nstale: off\n{ENTRY}"),
    );
    w.sync();
    let o = w.check(&[]);
    assert_eq!(o.status.code(), Some(0));
    assert!(!text(&o.stderr).contains("stale"), "{}", text(&o.stderr));

    // A file that doesn't read fails with its line.
    w.f.write(
        "cartograph.overrides.yaml",
        "version: 1\ncorrections:\n  - action: remove\n    why: x\n",
    );
    let o = w.check(&[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("line 4"), "{}", text(&o.stderr));
}
