//! Matching of event channels (XS-16): pure, no database.
//!
//! A channel is a (broker kind, name). Both ends of a pair name it as a literal (exact), through
//! an environment value (likely), with a broker wildcard on the subscribing side (likely), or only
//! by a prefix or the name of a constant (guessed). Wildcards are matched segment by segment by
//! hand: no regex is built from a topic that came from code or configuration.

use crate::links::confidence::EvidenceType as Ev;

/// The longest name kept; a longer one is cut.
pub const MAX_NAME: usize = 255;

/// How a broker's subscription patterns read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wild {
    /// Topics are exact names (Kafka, BullMQ, SQS, SNS).
    None,
    /// Dot-separated words: `*` is one word, `#` is zero or more (AMQP topic exchanges).
    Amqp,
    /// Dot-separated tokens: `*` is one token, `>` is one or more and last (NATS).
    Nats,
    /// A glob over the whole string: `*` is anything, `?` one character (Redis `PSUBSCRIBE`).
    Glob,
}

fn wild(broker: &str) -> Wild {
    match broker {
        "amqp" => Wild::Amqp,
        "nats" => Wild::Nats,
        "redis" => Wild::Glob,
        _ => Wild::None,
    }
}

/// Whether `name` is a subscription pattern rather than one channel.
pub fn is_pattern(broker: &str, name: &str) -> bool {
    match wild(broker) {
        Wild::None => false,
        Wild::Amqp => name.split('.').any(|s| s == "*" || s == "#"),
        Wild::Nats => name.split('.').any(|s| s == "*" || s == ">"),
        Wild::Glob => name.contains(['*', '?']),
    }
}

/// Whether the subscription `pattern` hears the channel `name` on this broker.
pub fn matches(broker: &str, pattern: &str, name: &str) -> bool {
    match wild(broker) {
        Wild::None => pattern == name,
        Wild::Glob => glob(pattern.as_bytes(), name.as_bytes()),
        w => {
            let p: Vec<&str> = pattern.split('.').collect();
            let n: Vec<&str> = name.split('.').collect();
            words(w, &p, &n)
        }
    }
}

/// Dot-separated patterns by dynamic programming: `ok[j]` is whether the words so far match the
/// first `j` words of the name.
fn words(w: Wild, pattern: &[&str], name: &[&str]) -> bool {
    let mut ok = vec![false; name.len() + 1];
    ok[0] = true;
    for (i, p) in pattern.iter().enumerate() {
        let mut next = vec![false; name.len() + 1];
        match (*p, w) {
            ("#", Wild::Amqp) => {
                // Zero or more words: reachable from any earlier column.
                let mut any = false;
                for j in 0..=name.len() {
                    any |= ok[j];
                    next[j] = any;
                }
            }
            (">", Wild::Nats) => {
                // One or more words, and only as the last token.
                if i + 1 != pattern.len() {
                    return false;
                }
                let mut any = false;
                for j in 1..=name.len() {
                    any |= ok[j - 1];
                    next[j] = any;
                }
            }
            ("*", _) => next[1..].copy_from_slice(&ok[..name.len()]),
            (lit, _) => {
                for j in 1..=name.len() {
                    next[j] = ok[j - 1] && name[j - 1] == lit;
                }
            }
        }
        ok = next;
    }
    ok[name.len()]
}

/// `*` and `?` over bytes, with the usual two-pointer backtrack to the last `*`.
fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < s.len() {
        if i < p.len() && p[i] == b'*' {
            star = Some((i, j));
            i += 1;
        } else if i < p.len() && (p[i] == b'?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|b| *b == b'*')
}

/// Whether an AsyncAPI channel address (`orders.{status}`, `orders/{id}`) names the channel: its
/// `{parameter}` words are any one word.
pub fn address_matches(address: &str, name: &str) -> bool {
    let words = |s: &str| s.split(['.', '/']).map(str::to_owned).collect::<Vec<_>>();
    let (a, n) = (words(address), words(name));
    a.len() == n.len()
        && a.iter()
            .zip(&n)
            .all(|(a, n)| a == n || (a.starts_with('{') && a.ends_with('}')))
}

/// The name a channel is kept under: a queue URL by its queue name, an ARN by its resource, so the
/// account id and the query string never reach the index, and a long name cut.
pub fn channel_name(raw: &str) -> String {
    let t = raw.trim();
    let name = if t.contains("://") {
        let path = t.split(['?', '#']).next().unwrap_or(t);
        let rest = path.split_once("://").map_or(path, |x| x.1);
        let tail = rest.split_once('/').map_or("", |x| x.1);
        tail.trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(rest)
    } else if t.starts_with("arn:") {
        t.rsplit(':').next().unwrap_or(t)
    } else {
        t
    };
    name.chars().take(MAX_NAME).collect()
}

/// How one end names its channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Name {
    /// A string the code spells out.
    Literal(String),
    /// A value an environment gives.
    Config(String),
    /// The start of a name the code builds: `orders.${region}`.
    Prefix(String),
    /// An expression naming a constant, as the source writes it.
    Constant(String),
}

/// The evidence that a subscriber named `sub` hears a publisher named `publ` on `broker`, if it
/// does. Two constants are one only when they are spelled alike.
pub fn pair(broker: &str, publ: &Name, sub: &Name) -> Option<Ev> {
    use Name::{Config, Constant, Literal, Prefix};
    match (publ, sub) {
        (Literal(p) | Config(p), Literal(s) | Config(s)) => {
            let exact = p == s;
            if !exact && !(is_pattern(broker, s) && matches(broker, s, p)) {
                return None;
            }
            Some(if !exact {
                Ev::ChannelWildcard
            } else if matches!(publ, Config(_)) || matches!(sub, Config(_)) {
                Ev::ChannelViaConfig
            } else {
                Ev::ChannelLiteral
            })
        }
        (Prefix(p), Literal(s) | Config(s) | Prefix(s)) | (Literal(s) | Config(s), Prefix(p)) => {
            (s.starts_with(p.as_str()) || p.starts_with(s.as_str())).then_some(Ev::ChannelNameMatch)
        }
        (Constant(a), Constant(b)) => (a == b).then_some(Ev::ChannelNameMatch),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn amqp_words() {
        for (p, n, want) in [
            ("order.*", "order.paid", true),
            ("order.*", "order.paid.late", false),
            ("order.#", "order", true),
            ("order.#", "order.paid.late", true),
            ("#", "a.b.c", true),
            ("*.paid", "order.paid", true),
            ("*.paid", "paid", false),
            ("a.#.z", "a.z", true),
            ("a.#.z", "a.b.c.z", true),
            ("a.#.z", "a.b.c", false),
        ] {
            assert_eq!(matches("amqp", p, n), want, "{p} {n}");
        }
    }

    #[test]
    fn nats_tokens() {
        for (p, n, want) in [
            ("order.>", "order.paid", true),
            ("order.>", "order", false),
            ("order.>", "order.a.b", true),
            ("order.*", "order.a.b", false),
            ("*.paid", "order.paid", true),
            (">", "a", true),
            ("a.>.c", "a.b.c", false),
        ] {
            assert_eq!(matches("nats", p, n), want, "{p} {n}");
        }
    }

    #[test]
    fn redis_globs_and_exact_brokers() {
        assert!(matches("redis", "order:*", "order:paid"));
        assert!(matches("redis", "o?der:*:late", "order:a:b:late"));
        assert!(!matches("redis", "order:*", "orders"));
        assert!(matches("kafka", "order.paid", "order.paid"));
        assert!(!matches("kafka", "order.*", "order.paid"));
        assert!(!is_pattern("kafka", "order.*"));
    }

    #[test]
    fn addresses_and_names() {
        assert!(address_matches("orders.{status}", "orders.paid"));
        assert!(!address_matches("orders.{status}", "orders.paid.late"));
        assert!(address_matches("orders/{id}/paid", "orders/7/paid"));
        assert_eq!(
            channel_name("https://sqs.eu-west-1.amazonaws.com/123456789012/orders?sig=abc"),
            "orders"
        );
        assert_eq!(
            channel_name("arn:aws:sns:eu-west-1:123456789012:order-paid"),
            "order-paid"
        );
        assert_eq!(channel_name("order.paid"), "order.paid");
    }

    #[test]
    fn the_three_levels() {
        use Name::*;
        let lit = |s: &str| Literal(s.into());
        assert_eq!(
            pair("kafka", &lit("order.paid"), &lit("order.paid")),
            Some(Ev::ChannelLiteral)
        );
        assert_eq!(
            pair("kafka", &lit("order.paid"), &Config("order.paid".into())),
            Some(Ev::ChannelViaConfig)
        );
        assert_eq!(
            pair("amqp", &lit("order.paid"), &lit("order.*")),
            Some(Ev::ChannelWildcard)
        );
        assert_eq!(
            pair("kafka", &Prefix("order.".into()), &lit("order.paid")),
            Some(Ev::ChannelNameMatch)
        );
        assert_eq!(
            pair(
                "kafka",
                &Constant("T.PAID".into()),
                &Constant("T.PAID".into())
            ),
            Some(Ev::ChannelNameMatch)
        );
        assert_eq!(pair("kafka", &lit("a"), &lit("b")), None);
    }

    fn word() -> impl Strategy<Value = String> {
        "[a-z]{1,4}"
    }

    proptest! {
        // Whatever the pattern and the name look like, matching ends and never panics.
        #[test]
        fn matching_is_total(p in ".{0,24}", n in ".{0,24}") {
            for b in ["amqp", "nats", "redis", "kafka"] {
                let _ = matches(b, &p, &n);
            }
        }

        // A name is heard by itself, by `#` or `>` for any suffix, and by `*` for any one word.
        #[test]
        fn a_name_is_heard_by_what_generalises_it(ws in prop::collection::vec(word(), 1..6), at in 0usize..6) {
            let name = ws.join(".");
            prop_assert!(matches("amqp", &name, &name));
            prop_assert!(matches("amqp", "#", &name));
            prop_assert!(matches("nats", ">", &name));
            let at = at % ws.len();
            let mut star = ws.clone();
            star[at] = "*".into();
            prop_assert!(matches("amqp", &star.join("."), &name));
            prop_assert!(matches("nats", &star.join("."), &name));
            let mut tail = ws[..at].to_vec();
            tail.push("#".into());
            prop_assert!(matches("amqp", &tail.join("."), &name));
            if at + 1 < ws.len() {
                let mut rest = ws[..=at].to_vec();
                rest.push(">".into());
                prop_assert!(matches("nats", &rest.join("."), &name));
            }
            // Fewer or more words than a wildcard-free pattern has never match.
            let mut longer = ws.clone();
            longer.push("x".into());
            prop_assert!(!matches("amqp", &longer.join("."), &name));
            prop_assert!(!matches("nats", &name, &longer.join(".")));
        }
    }
}
