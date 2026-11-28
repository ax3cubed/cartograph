//! XS-16: the fixture's `order.paid` publisher links to its consumer with the right level in each
//! of the three cases (literal on both ends, one end through config, a broker wildcard); a prefix
//! is guessed; the link runs from the subscriber to the publisher; the channel's reverse index and
//! an AsyncAPI operation show both sides; a sync equals a rebuild; an index with no event fact
//! runs no event pass.

use std::path::Path;

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

const COMPOSE: &str = "services:
  billing:
    build: ./billing
    environment:
      REFUND_TOPIC: order.refunded
  orders:
    build: ./orders
";
/// Publishes `order.paid` as a literal.
const PAY: &str = "import { Kafka } from \"kafkajs\";
export const paid = (p) => producer.send({ topic: \"order.paid\", messages: [p] });
";
/// Publishes `order.refunded` through `REFUND_TOPIC`.
const REFUND: &str = "import { Kafka } from \"kafkajs\";
const REFUND = process.env.REFUND_TOPIC;
export const refunded = (p) => producer.send({ topic: REFUND, messages: [p] });
";
/// Publishes to `audit.` and a kind it only knows at run time.
const AUDIT: &str = "import { Kafka } from \"kafkajs\";
export const audited = (p) => producer.send({ topic: `audit.${p.kind}`, messages: [p] });
";
/// Publishes `order.shipped` to an AMQP exchange.
const SHIP: &str = "const amqp = require(\"amqplib\");
exports.shipped = (c, m) => channel.publish(\"orders\", \"order.shipped\", m);
";
const PAID: &str = "import { Kafka } from \"kafkajs\";
await consumer.subscribe({ topic: \"order.paid\" });
";
const REFUNDED: &str = "import { Kafka } from \"kafkajs\";
await consumer.subscribe({ topic: \"order.refunded\" });
";
const LOGINS: &str = "import { Kafka } from \"kafkajs\";
await consumer.subscribe({ topic: \"audit.login\" });
";
/// A queue bound to every `order.*` routing key.
const ALL: &str = "const amqp = require(\"amqplib\");
channel.bindQueue(\"audit\", \"orders\", \"order.*\");
";
const ASYNC: &str = "asyncapi: 2.6.0
info: {title: Billing events, version: '1'}
channels:
  order.paid:
    subscribe:
      operationId: announcePaid
      summary: Billing announces a paid order
";

fn workspace(f: &Fixture, dir: &Path) -> Workspace {
    let svc = |name: &str| ServiceSpec {
        name: name.into(),
        repo: "repo".into(),
        path: Some(name.into()),
    };
    Workspace {
        dir: dir.to_owned(),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        services: vec![svc("billing"), svc("orders")],
        ..Default::default()
    }
}

fn run(w: &Workspace, cache: &mut FactCache) {
    match sync(w, &SyncOptions::default(), cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
}

fn q(ix: &Index, sql: &str) -> Vec<String> {
    ix.conn
        .prepare(sql)
        .unwrap()
        .query_map([], |r| {
            let n = r.as_ref().column_count();
            Ok((0..n)
                .map(|i| match r.get_ref(i).unwrap() {
                    cartograph_store::rusqlite::types::ValueRef::Null => "-".to_owned(),
                    cartograph_store::rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                    cartograph_store::rusqlite::types::ValueRef::Real(v) => format!("{v:.2}"),
                    cartograph_store::rusqlite::types::ValueRef::Text(t) => {
                        String::from_utf8_lossy(t).into_owned()
                    }
                    cartograph_store::rusqlite::types::ValueRef::Blob(_) => "blob".to_owned(),
                })
                .collect::<Vec<_>>()
                .join(" "))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

const PATH: &str =
    "(SELECT p.path FROM file_path p WHERE p.file = n.file ORDER BY p.from_seq DESC LIMIT 1)";

/// `subscriber -> publisher level from-to`, one row per link interval.
fn links(ix: &Index) -> Vec<String> {
    q(
        ix,
        &format!(
            "SELECT (SELECT {PATH} FROM node n WHERE n.id = l.src) || ' -> ' ||
                    (SELECT {PATH} FROM node n WHERE n.id = l.dst), l.level, i.from_seq,
                    coalesce(CAST(i.to_seq AS TEXT), 'open')
             FROM link l JOIN interval i ON i.link = l.id WHERE l.kind = 'event'
             ORDER BY 1, i.from_seq"
        ),
    )
}

fn fixture(d: &Path) -> Fixture {
    let mut f = Fixture::init(&d.join("repo"));
    f.write("docker-compose.yml", COMPOSE);
    f.write("billing/package.json", r#"{"name":"billing"}"#);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("billing/src/pay.ts", PAY);
    f.write("billing/src/refund.ts", REFUND);
    f.write("billing/src/audit.ts", AUDIT);
    f.write("billing/src/ship.ts", SHIP);
    f.commit("billing: publishes");
    f.write("orders/src/paid.ts", PAID);
    f.write("orders/src/refund.ts", REFUNDED);
    f.write("orders/src/audit.ts", LOGINS);
    f.write("orders/src/all.ts", ALL);
    f.commit("orders: subscribes");
    f.write("billing/asyncapi.yaml", ASYNC);
    f.commit("billing: the spec");
    f
}

#[test]
fn order_paid_links_to_its_consumer_with_the_level_each_case_earns() {
    let d = tempfile::tempdir().unwrap();
    let f = fixture(d.path());
    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();

    // Subscriber to publisher. `order.paid` is a literal on both ends (exact), `order.refunded`
    // comes through REFUND_TOPIC (likely), the wildcard hears `order.shipped` (likely) and
    // the `audit.` prefix reaches `audit.login` and nothing better (guessed).
    assert_eq!(
        links(&ix),
        [
            "orders/src/all.ts -> billing/src/ship.ts likely 1 open",
            "orders/src/audit.ts -> billing/src/audit.ts guessed 1 open",
            "orders/src/paid.ts -> billing/src/pay.ts exact 1 open",
            "orders/src/refund.ts -> billing/src/refund.ts likely 1 open",
        ]
    );
    // Both ends of each link are its evidence.
    assert_eq!(
        q(
            &ix,
            "SELECT count(*) FROM evidence e JOIN link l ON l.id = e.link WHERE l.kind = 'event'"
        ),
        ["8"]
    );

    // The reverse index: a channel name to both sides.
    assert_eq!(
        q(
            &ix,
            "SELECT broker, name, role, line FROM channel_edge WHERE name = 'order.paid'
             ORDER BY role"
        ),
        ["kafka order.paid publish 2", "kafka order.paid subscribe 2"]
    );

    // The AsyncAPI channel shows its publisher and its subscriber.
    assert_eq!(
        q(
            &ix,
            "SELECT c.kind, o.method, o.path, o.operation_id FROM operation o
             JOIN contract c ON c.id = o.contract"
        ),
        ["events publish order.paid announcePaid"]
    );
    assert_eq!(
        q(
            &ix,
            &format!(
                "SELECT b.kind, b.template, (SELECT {PATH} FROM node n WHERE n.file = b.file) FROM binding b
                 WHERE b.operation IS NOT NULL ORDER BY b.kind"
            )
        ),
        [
            "publishes order.paid billing/src/pay.ts",
            "subscribes order.paid orders/src/paid.ts"
        ]
    );
}

#[test]
fn a_sync_equals_a_rebuild_and_the_value_closes_the_link() {
    let d = tempfile::tempdir().unwrap();
    let mut f = fixture(d.path());
    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    run(&w, &mut cache);
    // The consumer is renamed away from the channel: the link closes at that commit.
    f.write(
        "orders/src/paid.ts",
        &PAID.replace("order.paid", "order.settled"),
    );
    f.commit("orders: listens to another channel");
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    let after = links(&ix);
    assert_eq!(
        after,
        [
            "orders/src/all.ts -> billing/src/ship.ts likely 1 open",
            "orders/src/audit.ts -> billing/src/audit.ts guessed 1 open",
            "orders/src/paid.ts -> billing/src/pay.ts exact 1 3",
            "orders/src/refund.ts -> billing/src/refund.ts likely 1 open",
        ]
    );
    let d2 = tempfile::tempdir().unwrap();
    let w2 = workspace(&f, &d2.path().join("ws"));
    let mut cache2 = FactCache::open(&d2.path().join("facts.db")).unwrap();
    run(&w2, &mut cache2);
    assert_eq!(links(&Index::open_read(&w2.index_path()).unwrap()), after);
}

#[test]
fn an_index_with_no_event_fact_has_no_event_rows() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("orders/src/a.ts", "export const a = 1;\n");
    f.commit("orders");
    let w = workspace(&f, &d.path().join("ws"));
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    run(&w, &mut cache);
    let ix = Index::open_read(&w.index_path()).unwrap();
    assert_eq!(q(&ix, "SELECT count(*) FROM channel_edge"), ["0"]);
    assert_eq!(links(&ix), Vec::<String>::new());
}
