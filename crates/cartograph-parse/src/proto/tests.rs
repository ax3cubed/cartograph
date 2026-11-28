use proptest::prelude::*;

use super::*;
use crate::{BlobFacts, Extractor};

/// In the shape of googleapis' `google/pubsub/v1/pubsub.proto`, cut down: a package, imports of
/// well-known and sibling files, options, nested messages, a oneof, a map, every rpc kind and an
/// `google.api.http` option block.
const PUBSUB: &str = r#"// Copyright 2024 Example LLC. Licensed under the Apache License, Version 2.0.
syntax = "proto3";

package google.pubsub.v1;

import "google/api/annotations.proto";
import "google/api/field_behavior.proto";
import "google/protobuf/timestamp.proto";
import "google/pubsub/v1/schema.proto";

option go_package = "cloud.google.com/go/pubsub/apiv1/pubsubpb;pubsubpb";

// The service that an application uses to manipulate topics.
service Publisher {
  option (google.api.default_host) = "pubsub.googleapis.com";

  rpc CreateTopic(Topic) returns (Topic) {
    option (google.api.http) = {
      put: "/v1/{name=projects/*/topics/*}"
      body: "*"
    };
  }

  rpc Publish(PublishRequest) returns (PublishResponse) {
    option (google.api.http) = { post: "/v1/{topic=projects/*/topics/*}:publish" body: "*" };
  }
}

service Subscriber {
  rpc StreamingPull(stream StreamingPullRequest) returns (stream StreamingPullResponse);
  rpc Upload(stream PublishRequest) returns (PublishResponse);
  rpc Watch(Topic) returns (stream PublishResponse);
}

message Topic {
  string name = 1 [(google.api.field_behavior) = REQUIRED];
  map<string, string> labels = 2;
  google.protobuf.Timestamp created = 3;
  oneof retention {
    int64 days = 4;
    .google.pubsub.v1.Policy policy = 5;
  }
  message Policy {
    repeated string allowed = 1;
  }
  enum State { STATE_UNSPECIFIED = 0; ACTIVE = 1; }
}

message PublishRequest {
  string topic = 1;
  repeated PubsubMessage messages = 2;
}
message PublishResponse { repeated string message_ids = 1; }
message PubsubMessage { bytes data = 1; }
message StreamingPullRequest { string subscription = 1; }
message StreamingPullResponse { optional int32 count = 1; }
"#;

fn read(src: &str) -> Contract {
    super::read(src.as_bytes()).expect("parses")
}

#[test]
fn golden_googleapis_sample() {
    let c = read(PUBSUB);
    let mut out = format!("{} {} files={:?}\n", c.title, c.version, c.files);
    for o in &c.operations {
        out += &format!(
            "{} {} id={:?} {:?} line {}\n",
            o.method, o.path, o.operation_id, o.summary, o.line
        );
    }
    for s in &c.schemas {
        out += &format!("message {}\n", s.name);
        for f in &s.fields {
            out += &format!(
                "  {} {}{}\n",
                f.name,
                f.ty,
                if f.required { " !" } else { "" }
            );
        }
    }
    insta::assert_snapshot!(out);
}

#[test]
fn facts_round_trip_to_a_contract() {
    let BlobFacts::Facts(f) = ManifestProto.extract(KIND, PUBSUB.as_bytes()) else {
        panic!("facts");
    };
    let mut c = openapi::from_facts(&f).expect("a spec fact");
    let mut want = read(PUBSUB);
    c.schemas.sort_by(|a, b| a.name.cmp(&b.name));
    want.schemas.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(c, want);
}

#[test]
fn only_proto_paths_are_handled() {
    assert_eq!(ManifestProto.handles("a/b.proto"), Some(KIND));
    assert_eq!(ManifestProto.handles("a/b.yaml"), None);
    assert!(crate::tracked("api/orders.proto"));
}

#[test]
fn a_file_without_a_package_has_no_prefix() {
    let c = read("syntax = \"proto3\"; service S { rpc M(A) returns (B); }");
    assert_eq!(c.operations[0].path, "/S/M");
    assert!(c.title.is_empty());
}

#[test]
fn imports_resolve_through_roots_and_stay_inside() {
    let files = ["proto/a/b.proto", "c/d.proto"];
    let mut has = |p: &str| files.contains(&p);
    let roots = vec!["proto".to_owned(), String::new()];
    assert_eq!(
        resolve("a/b.proto", &roots, &mut has),
        Ok("proto/a/b.proto".into())
    );
    assert_eq!(
        resolve("c/d.proto", &roots, &mut has),
        Ok("c/d.proto".into())
    );
    assert_eq!(resolve("a/x.proto", &roots, &mut has), Err("missing"));
    assert_eq!(resolve("../c/d.proto", &roots, &mut has), Err("outside"));
    assert_eq!(resolve("/etc/passwd", &roots, &mut has), Err("outside"));
    assert_eq!(resolve("a/../../x.proto", &roots, &mut has), Err("outside"));
    assert_eq!(
        resolve("google/protobuf/any.proto", &roots, &mut has),
        Err("bundled")
    );
    let deep = "a/".repeat(MAX_IMPORT_SEGMENTS + 1) + "x.proto";
    assert_eq!(resolve(&deep, &roots, &mut has), Err("deep"));
}

#[test]
fn buf_roots_name_modules_and_stay_inside() {
    let v2 =
        b"version: v2\nmodules:\n  - path: proto\n  - path: ../../escape\n  - path: shared/api\n";
    assert_eq!(buf_roots("svc", v2), ["svc/proto", "svc/shared/api"]);
    assert_eq!(
        buf_roots("svc", b"version: v1\nname: buf.build/a/b\n"),
        ["svc"]
    );
    assert_eq!(buf_roots("", b"directories: [a, b]\n"), ["a", "b"]);
    assert!(buf_roots("", b"{{{{").is_empty());
    assert_eq!(ancestors("a/b"), ["a/b", "a", ""]);
}

#[test]
fn caps_hold() {
    let many = "import \"a.proto\";\n".repeat(10) + &"message M { string a = 1; }\n".repeat(3);
    let c = read(&many);
    assert_eq!(c.files, ["a.proto"]);
    // A message nested past the depth cap stops the walk, it doesn't overflow the stack.
    let deep = "message M {".repeat(5_000) + &"}".repeat(5_000);
    assert!(read(&deep).schemas.len() <= MAX_DEPTH + 2);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// Fuzz target: no bytes panic the extractor, and the same bytes give the same facts.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let a = ManifestProto.extract(KIND, &bytes).encode();
        prop_assert_eq!(&a, &ManifestProto.extract(KIND, &bytes).encode());
    }

    /// Token soup of the grammar's own words reaches deeper than random bytes do.
    #[test]
    fn proto_like_text_never_panics(src in r#"(syntax|package|import|public|service|rpc|returns|stream|message|enum|oneof|map|repeated|optional|option|reserved|extend|group|=|;|\{|\}|\(|\)|<|>|,|\.|"|'|[a-zA-Z]+|[0-9]+| |\n|/\*|\*/|//)*"#) {
        let _ = ManifestProto.extract(KIND, src.as_bytes());
    }

    /// Resolution is confined whatever the import says: a result is always a path with no `..`.
    #[test]
    fn resolved_imports_never_climb(import in "[a-z./:\\\\]{0,40}") {
        let mut yes = |_: &str| true;
        if let Ok(p) = resolve(&import, &[String::new(), "x".into()], &mut yes) {
            prop_assert!(!p.split('/').any(|s| s == ".." || s.is_empty()) && !p.starts_with('/'));
        }
    }
}
