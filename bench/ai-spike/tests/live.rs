//! Real provider calls, on demand only. Each reads its key from the environment and is ignored
//! by default:
//!   OPENAI_KEY=… ANTHROPIC_KEY=… GEMINI_KEY=… cargo test -p ai-spike --test live -- --ignored
//! Ollama and an OpenAI-compatible server use LIVE_OLLAMA_URL / LIVE_COMPAT_URL (+ LIVE_COMPAT_MODEL).

use ai_spike::*;
use genai::adapter::AdapterKind;
use genai::resolver::{AuthData, Endpoint};
use genai::{ModelIden, ServiceTarget};

async fn live(kind: AdapterKind, base: &str, model: &str, key: &str) {
    let t = ServiceTarget {
        endpoint: Endpoint::from_owned(base.to_string()),
        auth: AuthData::from_single(key),
        model: ModelIden::new(kind, model),
    };
    let out = run_stream(&our_client(), t, request()).await.unwrap();
    eprintln!("{kind:?} {model}: {out:?}");
    assert!(out.usage.is_some(), "no usage in the stream");
    assert!(
        !out.tool_calls.is_empty() || !out.text.is_empty(),
        "neither text nor tool calls"
    );
}

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("set {k}"))
}

#[tokio::test]
#[ignore]
async fn openai() {
    live(
        AdapterKind::OpenAI,
        "https://api.openai.com/v1/",
        "o4-mini",
        &env("OPENAI_KEY"),
    )
    .await;
}

#[tokio::test]
#[ignore]
async fn anthropic() {
    live(
        AdapterKind::Anthropic,
        "https://api.anthropic.com/v1/",
        "claude-sonnet-4-5",
        &env("ANTHROPIC_KEY"),
    )
    .await;
}

#[tokio::test]
#[ignore]
async fn gemini() {
    live(
        AdapterKind::Gemini,
        "https://generativelanguage.googleapis.com/v1beta/",
        "gemini-2.5-flash",
        &env("GEMINI_KEY"),
    )
    .await;
}

#[tokio::test]
#[ignore]
async fn ollama() {
    live(AdapterKind::Ollama, &env("LIVE_OLLAMA_URL"), "qwen3", "").await;
}

#[tokio::test]
#[ignore]
async fn openai_compatible() {
    live(
        AdapterKind::OpenAI,
        &env("LIVE_COMPAT_URL"),
        &env("LIVE_COMPAT_MODEL"),
        "none",
    )
    .await;
}
