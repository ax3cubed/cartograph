use proptest::prelude::*;

use super::*;

/// Known-shaped secrets, after the vectors in gitleaks' rule tests: (rule, text, the secret).
const VECTORS: &[(&str, &str, &str)] = &[
    (
        "aws-access-key",
        "AWS_KEY=AKIAIOSFODNN7EXAMPLE",
        "AKIAIOSFODNN7EXAMPLE",
    ),
    (
        "aws-secret-key",
        "aws_secret_access_key = \"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    ),
    (
        "github-token",
        "token: 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",
        "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",
    ),
    (
        "github-pat",
        "github_pat_11ABCDEFG0abcdefghijkl_mnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXY",
        "github_pat_11ABCDEFG0abcdefghijkl_mnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXY",
    ),
    (
        "gitlab-pat",
        "glpat-1234567890abcdefghij",
        "glpat-1234567890abcdefghij",
    ),
    (
        "slack-token",
        "xoxb-263594206564-2343594206574-FGqddMF8t08v8N7Oq4i57vs1MBB8",
        "xoxb-263594206564-2343594206574-FGqddMF8t08v8N7Oq4i57vs1MBB8",
    ),
    (
        "stripe-key",
        "sk_live_51HqLyjWDarjtT1zdp7dc0a1b2c3d",
        "sk_live_51HqLyjWDarjtT1zdp7dc0a1b2c3d",
    ),
    (
        "google-api-key",
        "key=AIzaSyA-1234567890abcdefghijklmnopqrstu",
        "AIzaSyA-1234567890abcdefghijklmnopqrstu",
    ),
    (
        "npm-token",
        "//registry.npmjs.org/:_authToken=npm_1234567890abcdefghijklmnopqrstuvwxyz",
        "npm_1234567890abcdefghijklmnopqrstuvwxyz",
    ),
    (
        "jwt",
        "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
    ),
    (
        "private-key",
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1234abcd\n-----END RSA PRIVATE KEY-----",
        "MIIEowIBAAKCAQEA1234abcd",
    ),
    ("url-credentials", "postgres://user:pw@db/app", "user:pw"),
    (
        "generic-assignment",
        "const password = 'hunter2hunter2';",
        "hunter2hunter2",
    ),
];

#[test]
fn every_vector_is_found_by_its_rule_and_masked() {
    let s = Scanner::builtin();
    for (rule, text, secret) in VECTORS {
        let hits = s.scan(text);
        assert!(hits.iter().any(|h| h.rule == *rule), "{rule}: {hits:?}");
        let masked = s.mask(text);
        assert!(!masked.contains(secret), "{rule}: {masked}");
        assert!(masked.contains(MASK), "{rule}: {masked}");
    }
}

#[test]
fn a_url_credential_is_masked_and_the_host_kept() {
    assert_eq!(
        mask("postgres://user:pw@db/app"),
        "postgres://[REDACTED]@db/app"
    );
}

#[test]
fn ordinary_code_text_is_left_alone() {
    for t in [
        "fetch(`http://orders:8080/orders/${id}`)",
        "app.get('/orders/:id', handler)",
        "token: string,",
        "import { getOrderByCustomerIdentifier } from './orders/queries'",
        "https://example.com/a/b?c=d",
        "7448606b36055f1e0b2c9d8a7f6e5d4c3b2a1908", // a commit sha: letters and digits, but hex
        "@babel/plugin-transform-class-properties",
        "const apiKey = process.env.API_KEY;",
    ] {
        assert_eq!(mask(t), t);
    }
}

#[test]
fn a_long_random_token_is_caught_by_entropy_alone() {
    let t = "see x9Qk2mZ8vL4pR7tY1wB5nC3dF6gH0jA2 for details";
    let m = mask(t);
    assert!(!m.contains("x9Qk2mZ8"), "{m}");
    assert_eq!(&*m, "see [REDACTED] for details");
    // Shorter than 20 characters is never checked.
    assert_eq!(mask("x9Qk2mZ8vL4pR7t1"), "x9Qk2mZ8vL4pR7t1");
}

#[test]
fn custom_rules_are_size_limited_and_compile_checked() {
    let ok = Scanner::new(&[("internal".into(), r"\bINT-[0-9]{6}\b".into())]).unwrap();
    assert_eq!(ok.mask("ticket INT-123456 here"), "ticket [REDACTED] here");
    assert!(Scanner::new(&[("bad".into(), "(".into())]).is_err());
    assert!(Scanner::new(&[("big".into(), "a".repeat(MAX_RULE_BYTES + 1))]).is_err());
    assert!(Scanner::new(&[("huge".into(), "(a{1000}){1000}".into())]).is_err());
}

#[test]
fn a_config_value_is_stored_only_as_a_locator() {
    use ConfigValue::{Locator, Opaque};
    let c = classify_config_value;
    assert_eq!(
        c("CATALOG_URL", "http://catalog:8080/v1"),
        Locator("http://catalog:8080/v1".into())
    );
    assert_eq!(
        c("DB_URL", "postgres://user:pw@db:5432/app?sslmode=disable#x"),
        Locator("postgres://db:5432/app".into())
    );
    assert_eq!(
        c("HOST", "orders.internal:8080"),
        Locator("orders.internal:8080".into())
    );
    assert_eq!(
        c("ORDERS_TOPIC", "orders.created"),
        Locator("orders.created".into())
    );
    // A key that smells of a credential, whatever the value.
    for k in [
        "API_KEY",
        "db_password",
        "AuthHeader",
        "GITHUB_TOKEN",
        "PRIVATE_URL",
        "SENTRY_DSN",
        "client_secret",
        "CREDENTIALS",
    ] {
        assert_eq!(c(k, "http://harmless/"), Opaque, "{k}");
    }
    // A value that isn't a locator.
    assert_eq!(c("GREETING", "hello there, world"), Opaque);
    assert_eq!(
        c(
            "BLOB",
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        ),
        Opaque
    );
    assert_eq!(c("X", "AKIAIOSFODNN7EXAMPLE"), Opaque);
    assert_eq!(c("X", "x9Qk2mZ8vL4pR7tY1wB5nC3dF6gH0jA2"), Opaque);
    assert_eq!(c("X", ""), Opaque);
}

proptest! {
    /// A URL goes in with userinfo, a query and a fragment and comes out with none.
    #[test]
    fn the_url_scrubber_drops_userinfo_query_and_fragment(
        user in "[a-z0-9]{1,8}", pw in "[ -~&&[^/@?#]]{0,12}",
        host in "[a-z]{1,8}(\\.[a-z]{1,5}){0,2}", path in "(/[a-z0-9]{1,6}){0,3}",
        query in "[a-z0-9=&]{0,12}", frag in "[a-z0-9]{0,6}",
    ) {
        let url = format!("https://{user}:{pw}@{host}{path}?{query}#{frag}");
        let out = scrub_url(&url).unwrap();
        prop_assert_eq!(out, format!("https://{host}{path}"));
    }

    /// Whatever the input, the scrubber returns nothing or a URL with no userinfo, query or fragment.
    #[test]
    fn the_url_scrubber_never_panics_or_keeps_a_secret_part(s in "\\PC{0,60}") {
        if let Some(out) = scrub_url(&s) {
            prop_assert!(!out.contains('?') && !out.contains('#'));
            let after = out.split_once("://").unwrap().1;
            let authority = after.split('/').next().unwrap();
            prop_assert!(!authority.contains('@'));
        }
    }

    /// No planted secret survives a mask, wherever it sits in other text.
    #[test]
    fn a_planted_secret_never_survives_a_mask(
        i in 0..VECTORS.len(), before in "[a-z ]{0,10}", after in "[a-z ]{0,10}",
    ) {
        let (_, text, secret) = VECTORS[i];
        let masked = mask(&format!("{before}{text}{after}")).into_owned();
        prop_assert!(!masked.contains(secret));
        // Masking is idempotent.
        prop_assert_eq!(mask(&masked).into_owned(), masked);
    }

    #[test]
    fn mask_never_panics(s in "\\PC{0,300}") {
        let _ = mask(&s);
    }
}
