//! Runs every AI-01 check against the in-process mock and prints a JSON report.
//!   cargo run -p ai-spike [-- --json out.json] [-- --skip-keyring]

use std::time::Duration;

use ai_spike::*;
use genai::Client;
use genai::adapter::AdapterKind;
use genai::resolver::{AuthData, AuthResolver, Endpoint};
use serde_json::{Value, json};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let json_out = args
        .iter()
        .position(|a| a == "--json")
        .and_then(|i| args.get(i + 1).cloned());
    // Upgrade check: `--keyring-step set` from one build, then `get` and `delete` from a
    // rebuilt binary (build with SPIKE_BUILD=<anything> to change its bytes).
    if let Some(i) = args.iter().position(|a| a == "--keyring-step") {
        let step = args.get(i + 1).map(String::as_str).unwrap_or("");
        println!(
            "build={} step={step} -> {:?}",
            option_env!("SPIKE_BUILD").unwrap_or("a"),
            keyring_step(step)
        );
        return;
    }
    let mock = start_mock().await;
    let mut report = serde_json::Map::new();

    // -- per provider: streaming text, parallel tool calls, usage, request body
    let client = our_client();
    let mut prov = serde_json::Map::new();
    for (name, kind, path, model) in providers() {
        let before = mock.reqs.lock().unwrap().len();
        let res = run_stream(
            &client,
            target(&mock.base, kind, path, model, "mock-key"),
            request(),
        )
        .await;
        let rec = mock.reqs.lock().unwrap().get(before).cloned();
        let entry = match (res, rec) {
            (Ok(o), Some(r)) => json!({
                "path": r.path,
                "base_url_used": r.path.starts_with(&format!("/{path}")),
                "key_header": r.header(key_header(kind)),
                "streamed_text": o.text, "text_chunks": o.text_chunks,
                "tool_call_chunks": o.tool_call_chunks,
                "tool_calls": o.tool_calls.iter().map(|(id, n, a)| json!([id, n, a])).collect::<Vec<_>>(),
                "usage": serde_json::to_value(&o.usage).unwrap(),
                "request_body": r.body,
            }),
            (Err(e), r) => json!({"error": e, "path": r.map(|r| r.path)}),
            (Ok(_), None) => json!({"error": "mock saw no request"}),
        };
        prov.insert(name.into(), entry);
    }
    report.insert("providers".into(), Value::Object(prov));

    // -- cancellation by dropping the stream
    report.insert(
        "cancel_by_drop_closes_connection".into(),
        json!(cancel_by_drop(&mock).await),
    );

    // -- ambient env vars and the default auth resolver
    report.insert("env".into(), env_checks(&mock).await);

    // -- keyring
    if !args.iter().any(|a| a == "--skip-keyring") {
        let kr = tokio::time::timeout(
            Duration::from_secs(20),
            tokio::task::spawn_blocking(keyring_roundtrip),
        )
        .await;
        report.insert(
            "keyring".into(),
            match kr {
                Ok(Ok(r)) => json!(r),
                Ok(Err(e)) => json!({"error": e.to_string()}),
                Err(_) => json!({"error": "timed out after 20 s (a Keychain dialog?)"}),
            },
        );
    }

    report.insert("acp_client_caps_default".into(), acp_client_caps_default());

    let text = serde_json::to_string_pretty(&Value::Object(report)).unwrap();
    println!("{text}");
    if let Some(p) = json_out {
        std::fs::write(p, &text).unwrap();
    }
}

async fn env_checks(mock: &Mock) -> Value {
    // SAFETY: no other task touches the environment while this runs.
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-ambient-from-env") };
    let ep = || Endpoint::from_owned(format!("{}/anthropic/v1/", mock.base));
    let last_key = |m: &Mock| {
        m.reqs
            .lock()
            .unwrap()
            .last()
            .and_then(|r| r.header("x-api-key").map(str::to_string))
    };

    // 1. Default client, endpoint override only: which key is sent?
    let c1 = Client::builder()
        .append_provider_config(AdapterKind::Anthropic, ep())
        .build()
        .unwrap();
    let r = c1
        .exec_chat_stream("claude-sonnet-4-5", request(), None)
        .await;
    let r1 = drained(r).await;
    let k1 = last_key(mock);

    // 2. An auth resolver that returns None (no key in our store): does genai fall back to env?
    let c2 = Client::builder()
        .append_provider_config(AdapterKind::Anthropic, ep())
        .with_auth_resolver(AuthResolver::from_resolver_fn(
            |_m: genai::ModelIden| -> Result<Option<AuthData>, genai::resolver::Error> { Ok(None) },
        ))
        .build()
        .unwrap();
    let r = c2
        .exec_chat_stream("claude-sonnet-4-5", request(), None)
        .await;
    let r2 = drained(r).await;
    let k2 = last_key(mock);

    // 3. An explicit ServiceTarget with our key: env must not be read.
    let c3 = our_client();
    let r = c3
        .exec_chat_stream(
            target(
                &mock.base,
                AdapterKind::Anthropic,
                "anthropic/v1/",
                "claude-sonnet-4-5",
                "sk-from-keystore",
            ),
            request(),
            None,
        )
        .await;
    let r3 = drained(r).await;
    let k3 = last_key(mock);
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };

    json!({
        "default_client_sends": k1, "default_ok": r1,
        "resolver_returning_none_sends": k2, "resolver_none_ok": r2,
        "explicit_service_target_sends": k3, "explicit_ok": r3,
    })
}

/// The request is sent when the stream is first polled, so read it to the end.
async fn drained(res: genai::Result<genai::chat::ChatStreamResponse>) -> bool {
    use futures::StreamExt;
    match res {
        Ok(r) => r.stream.all(|e| async move { e.is_ok() }).await,
        Err(_) => false,
    }
}
