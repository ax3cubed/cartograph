//! `cartograph check` and architecture rules (XS-22, H6): a `layer:web` service reads a table, so
//! the rule fails check; the baseline makes it pass; a fixed violation is reported; and a link that
//! is only guessed never fails.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use cartograph_discovery::manifest::Service;
use cartograph_discovery::rules::{MinConfidence, Rule, RuleType, Selector, Severity};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;

const MIGRATION: &str = "CREATE TABLE orders (id int primary key, total numeric);\n";
const REPO: &str = "export function load() {\n  return db.query(\"SELECT id FROM orders\");\n}\n";

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

fn rule(name: &str, severity: Severity) -> Rule {
    Rule {
        name: name.into(),
        kind: RuleType::Forbid,
        severity,
        from: Selector {
            tags: vec!["layer:web".into()],
            ..Selector::default()
        },
        to: Selector::default(),
        link: vec![cartograph_discovery::rules::LinkKind::Db],
        min_confidence: MinConfidence::Likely,
        reason: Some("the web layer goes through the API".into()),
    }
}

fn workspace(root: &Path, rules: Vec<Rule>) -> Ws {
    let data = root.join("data");
    let mut f = Fixture::init(&root.join("repo"));
    f.write(
        "docker-compose.yml",
        "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n",
    );
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    f.write("orders/migrations/001.sql", MIGRATION);
    f.write("web/src/repo.ts", REPO);
    f.commit("two services");
    let mut plan = cartograph_discovery::setup::plan(&f.dir, None).unwrap();
    let repo = plan.manifest.repos[0].name.clone();
    plan.manifest.services = [("orders", "layer:data"), ("web", "layer:web")]
        .map(|(n, t)| Service {
            name: n.into(),
            repo: repo.clone(),
            path: Some(n.into()),
            tags: vec![t.into()],
        })
        .into();
    plan.manifest.rules = rules;
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

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap()
}

#[test]
fn a_web_service_reading_a_table_fails_until_it_is_baselined() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let mut w = workspace(&root, vec![rule("web-not-db", Severity::Error)]);
    w.sync();

    let o = w.check(&["--json"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o.stderr));
    let err = text(&o.stderr);
    assert!(
        err.contains("web-not-db error: db web:web/src/repo.ts -> orders:orders"),
        "{err}"
    );
    let j = json(&o);
    assert_eq!(j["rules"][0]["name"], "web-not-db");
    assert_eq!(j["rules"][0]["violations"], 1);
    assert_eq!(j["rules"][0]["items"][0]["op"], "read orders");

    // Accepting what is there writes the file, and then check passes.
    let o = w.check(&["--update-baseline"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let file = std::fs::read_to_string(w.f.dir.join("cartograph.baseline.json")).unwrap();
    assert!(
        file.contains("\"version\": 1") && file.contains("web-not-db"),
        "{file}"
    );
    let o = w.check(&["--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(json(&o)["rules"][0]["baselined"], 1);
    assert_eq!(json(&o)["fixed"], serde_json::json!([]));

    // The web code stops reading the table: the baseline entry is reported as fixed.
    w.f.write("web/src/repo.ts", "export const x = 1;\n");
    w.f.commit("stop reading");
    w.sync();
    let o = w.check(&["--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(json(&o)["fixed"][0]["rule"], "web-not-db");
    assert_eq!(json(&o)["rules"][0]["violations"], 0);
}

#[test]
fn warn_prints_and_a_guessed_link_never_fails() {
    let d = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(d.path()).unwrap();
    let mut w = workspace(&root, vec![rule("soft", Severity::Warn)]);
    w.sync();
    let o = w.check(&[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert!(
        text(&o.stdout).contains("soft warn: db web:web/src/repo.ts"),
        "{}",
        text(&o.stdout)
    );

    // The same violation with an error rule, once the link is only a guess.
    let d2 = tempfile::tempdir().unwrap();
    let root2 = std::fs::canonicalize(d2.path()).unwrap();
    let mut w = workspace(&root2, vec![rule("hard", Severity::Error)]);
    w.sync();
    assert_eq!(w.check(&[]).status.code(), Some(1));
    let ix = w
        .data
        .join("workspaces")
        .join(&w.id)
        .join("index.cartograph");
    let conn = cartograph_store::rusqlite::Connection::open(ix).unwrap();
    conn.execute("UPDATE link SET level = 'guessed', score = 0.45", [])
        .unwrap();
    drop(conn);
    let o = w.check(&[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
}
