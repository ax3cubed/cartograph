//! Does genai read ambient provider env vars? Its own test binary, because it sets the
//! process environment.

use ai_spike::*;
use futures::StreamExt;
use genai::Client;
use genai::adapter::AdapterKind;
use genai::resolver::{AuthData, AuthResolver, Endpoint};

async fn sent_key(mock: &Mock, client: &Client, model: impl Into<genai::ModelSpec>) -> String {
    let res = client
        .exec_chat_stream(model, request(), None)
        .await
        .unwrap();
    // The request goes out on the first poll.
    res.stream.for_each(|_| async {}).await;
    let reqs = mock.reqs.lock().unwrap();
    reqs.last()
        .unwrap()
        .header("x-api-key")
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn default_auth_reads_env_and_only_an_explicit_target_avoids_it() {
    let mock = start_mock().await;
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-ambient-from-env") };
    let ep = Endpoint::from_owned(format!("{}/anthropic/v1/", mock.base));

    let default = Client::builder()
        .append_provider_config(AdapterKind::Anthropic, ep.clone())
        .build()
        .unwrap();
    assert_eq!(
        sent_key(&mock, &default, "claude-sonnet-4-5").await,
        "sk-ambient-from-env"
    );

    // A resolver that finds no key in our store still falls back to the env var.
    let none = Client::builder()
        .append_provider_config(AdapterKind::Anthropic, ep)
        .with_auth_resolver(AuthResolver::from_resolver_fn(
            |_m: genai::ModelIden| -> Result<Option<AuthData>, genai::resolver::Error> { Ok(None) },
        ))
        .build()
        .unwrap();
    assert_eq!(
        sent_key(&mock, &none, "claude-sonnet-4-5").await,
        "sk-ambient-from-env"
    );

    // An explicit ServiceTarget bypasses the resolver chain entirely.
    let t = target(
        &mock.base,
        AdapterKind::Anthropic,
        "anthropic/v1/",
        "claude-sonnet-4-5",
        "sk-from-keystore",
    );
    assert_eq!(sent_key(&mock, &our_client(), t).await, "sk-from-keystore");
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
}
