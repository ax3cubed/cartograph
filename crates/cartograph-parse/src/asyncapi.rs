//! AsyncAPI 2.x and 3.0 (XS-16) read into the contract model of [`crate::openapi`]: an operation is
//! a channel address the application publishes to or subscribes to.
//!
//! The two versions disagree about who the verbs describe. In 2.x, `publish` says what a client
//! may publish to the channel, so the application *receives* there; `subscribe` is what the
//! application *sends*. In 3.0, `send` and `receive` describe the application itself. The model
//! keeps the application's view, so 2.x is flipped: `publish` here always means the application
//! publishes, `subscribe` that it consumes.
//!
//! Only data is read, no reference leaves the document, and the caps of the OpenAPI parser apply.

use serde_json::Value;

use crate::openapi::{Contract, MAX_OPERATIONS, MAX_SUMMARY, Operation, pointer};

fn cut(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// The line of the first mention of `needle` in the source: 1 when the text isn't there.
fn line_of(text: &str, needle: &str) -> u32 {
    text.lines()
        .position(|l| l.contains(needle))
        .map_or(1, |i| i as u32 + 1)
}

fn push(c: &mut Contract, method: &str, address: &str, op: &Value, text: &str) {
    if c.operations.len() >= MAX_OPERATIONS {
        c.truncated = true;
        return;
    }
    c.operations.push(Operation {
        method: method.to_owned(),
        path: cut(address, 2048),
        operation_id: op
            .get("operationId")
            .and_then(Value::as_str)
            .map(|s| cut(s, 256)),
        summary: op
            .get("summary")
            .and_then(Value::as_str)
            .map(|s| cut(s, MAX_SUMMARY)),
        line: line_of(text, address),
    });
}

/// The contract of an AsyncAPI document; `None` for a version this reader doesn't know.
pub fn read(root: &Value, version: &str, bytes: &[u8]) -> Option<Contract> {
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let mut c = Contract {
        version: cut(version, 32),
        title: cut(
            root.pointer("/info/title")
                .and_then(Value::as_str)
                .unwrap_or(""),
            200,
        ),
        bases: vec![String::new()],
        events: true,
        ..Default::default()
    };
    let channels = root.get("channels").and_then(Value::as_object);
    if version.starts_with("2.") {
        let mut named: Vec<_> = channels.into_iter().flatten().collect();
        named.sort_by(|a, b| a.0.cmp(b.0));
        for (address, item) in named {
            // The other party's verb, flipped to the application's.
            for (verb, method) in [("publish", "subscribe"), ("subscribe", "publish")] {
                if let Some(op) = item.get(verb).filter(|o| o.is_object()) {
                    push(&mut c, method, address, op, text);
                }
            }
        }
    } else if version.starts_with("3.") {
        let mut ops: Vec<_> = root
            .get("operations")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .collect();
        ops.sort_by(|a, b| a.0.cmp(b.0));
        for (name, op) in ops {
            let method = match op.get("action").and_then(Value::as_str) {
                Some("send") => "publish",
                Some("receive") => "subscribe",
                _ => continue,
            };
            // The channel is a reference into this document, whose `address` is the topic; with no
            // address the channel's key is.
            let Some(key) = op
                .pointer("/channel/$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.strip_prefix('#'))
            else {
                continue;
            };
            let Some(ch) = pointer(root, key) else {
                continue;
            };
            let address = ch
                .get("address")
                .and_then(Value::as_str)
                .or_else(|| key.rsplit('/').next())
                .unwrap_or_default();
            if address.is_empty() {
                continue;
            }
            let mut o = op.clone();
            if o.get("operationId").is_none() {
                o["operationId"] = Value::String(name.clone());
            }
            push(&mut c, method, address, &o, text);
        }
    } else {
        return None;
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    fn ops(spec: &str) -> Vec<String> {
        let c = crate::openapi::parse(spec.as_bytes(), None)
            .unwrap()
            .unwrap();
        assert!(c.events);
        c.operations
            .iter()
            .map(|o| {
                format!(
                    "{} {} {} L{}",
                    o.method,
                    o.path,
                    o.operation_id.as_deref().unwrap_or("-"),
                    o.line
                )
            })
            .collect()
    }

    const V2: &str = "asyncapi: 2.6.0
info: {title: Orders, version: '1'}
channels:
  order.paid:
    publish:
      operationId: onPaid
      summary: Payments announce an order is paid
  order.shipped:
    subscribe:
      operationId: sendShipped
";

    const V3: &str = "asyncapi: 3.0.0
info: {title: Orders, version: '1'}
channels:
  paid:
    address: order.paid
  shipped:
    address: order.shipped
operations:
  onPaid:
    action: receive
    channel: {$ref: '#/channels/paid'}
  sendShipped:
    action: send
    channel: {$ref: '#/channels/shipped'}
";

    /// The same service in both versions: the 2.x verbs are the other party's, so they flip.
    #[test]
    fn two_and_three_say_the_same_thing_with_opposite_verbs() {
        let a = ops(V2);
        let b = ops(V3);
        insta::assert_snapshot!(format!("2.x\n{}\n3.0\n{}", a.join("\n"), b.join("\n")));
        let strip = |v: &[String]| -> Vec<String> {
            v.iter()
                .map(|s| s.split(" L").next().unwrap().to_owned())
                .collect()
        };
        assert_eq!(strip(&a), strip(&b));
    }

    #[test]
    fn an_unknown_version_or_a_missing_channel_gives_nothing() {
        assert!(
            crate::openapi::parse(b"asyncapi: 1.2.0\nchannels: {}\n", None)
                .unwrap()
                .is_none()
        );
        let c = crate::openapi::parse(
            b"asyncapi: 3.0.0\noperations:\n  x: {action: send, channel: {$ref: '#/channels/nope'}}\n",
            None,
        )
        .unwrap()
        .unwrap();
        assert!(c.operations.is_empty());
    }
}
