//! XS-16 done-when: an AsyncAPI channel shows its publisher and its subscribers in the Contracts
//! read, and the event link between the two files is a link of kind `event` whose evidence is both
//! call sites.

use cartograph_api::types::{ConfidenceLevel, LinkKind, Method, OperationStatus, SpecKind};
use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_query::{At, Limits, Store};

const PAY: &str =
    "import { Kafka } from \"kafkajs\";\nproducer.send({ topic: \"order.paid\", messages: [] });\n";
const PAID: &str =
    "import { Kafka } from \"kafkajs\";\nconsumer.subscribe({ topic: \"order.paid\" });\n";
const SPEC: &str = "asyncapi: 3.0.0
info: {title: Billing events, version: '1'}
channels:
  paid: {address: order.paid}
  refunded: {address: order.refunded}
operations:
  announcePaid: {action: send, channel: {$ref: '#/channels/paid'}}
  announceRefund: {action: send, channel: {$ref: '#/channels/refunded'}}
";

#[test]
fn an_asyncapi_channel_shows_its_publisher_and_subscribers() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write("billing/package.json", r#"{"name":"billing"}"#);
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("billing/src/pay.ts", PAY);
    f.write("orders/src/paid.ts", PAID);
    f.write("billing/asyncapi.yaml", SPEC);
    f.commit("billing publishes, orders subscribes, the spec says so");

    let svc = |name: &str| ServiceSpec {
        name: name.into(),
        repo: "repo".into(),
        path: Some(name.into()),
    };
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        services: vec![svc("billing"), svc("orders")],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let store = Store::open_path(&w.index_path()).unwrap();
    let snap = store.snapshot(Limits::default()).unwrap();

    let specs = snap.contracts(&At::Now).unwrap();
    assert_eq!(specs.len(), 1);
    let c = &specs[0];
    assert!(matches!(c.kind, SpecKind::Events));
    assert_eq!(c.counts.operations, 2);
    let paid = c
        .operations
        .iter()
        .find(|o| o.path == "order.paid")
        .unwrap();
    assert!(matches!(paid.method, Method::Event));
    assert_eq!(paid.operation_id.as_deref(), Some("announcePaid"));
    assert_eq!(paid.status, OperationStatus::Sync);
    assert_eq!(paid.publishers.len(), 1);
    assert_eq!(paid.subscribers.len(), 1);
    assert_eq!(paid.subscribers[0].confidence.level, ConfidenceLevel::Exact);
    // The channel nobody publishes to is the spec's alone.
    let refunded = c
        .operations
        .iter()
        .find(|o| o.path == "order.refunded")
        .unwrap();
    assert_eq!(refunded.status, OperationStatus::SpecOnly);
    assert!(refunded.publishers.is_empty() && refunded.subscribers.is_empty());

    // The event link reads like the others: its call sites are its evidence.
    let detail = (1..40)
        .filter_map(|n| snap.link_detail(&format!("l{n}"), &At::Now).ok())
        .find(|d| d.link.kind == LinkKind::Event)
        .expect("the event link");
    assert_eq!(detail.link.confidence.level, ConfidenceLevel::Exact);
    assert_eq!(detail.evidence.caller.len(), 1);
    assert_eq!(detail.evidence.handler.len(), 1);
}
