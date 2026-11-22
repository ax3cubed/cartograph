//! AI-01 spike. Checks what decisions G3 (genai) and G4 (keyring) assume, against a mock
//! provider on 127.0.0.1, with no real keys. Throwaway: nothing here ships.

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Uri};
use futures::{StreamExt, stream};
use genai::adapter::AdapterKind;
use genai::chat::{
    CacheControl, ChatMessage, ChatOptions, ChatRequest, ChatStreamEvent, ContentPart,
    ReasoningEffort, Tool, Usage,
};
use genai::resolver::{AuthData, Endpoint};
use genai::{Client, ModelIden, ServiceTarget};
use serde_json::{Value, json};

// ---------------------------------------------------------------- mock provider

/// One request the mock received.
#[derive(Debug, Clone)]
pub struct Rec {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

impl Rec {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub struct Mock {
    pub base: String,
    pub reqs: Arc<Mutex<Vec<Rec>>>,
    /// Set when the client drops a held-open stream (cancellation check).
    pub closed: Arc<AtomicBool>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Mock {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

struct CloseGuard(Arc<AtomicBool>);
impl Drop for CloseGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn sse(events: &[(Option<&str>, Value)], done: bool) -> Vec<String> {
    let mut out: Vec<String> = events
        .iter()
        .map(|(ev, data)| match ev {
            Some(e) => format!("event: {e}\ndata: {data}\n\n"),
            None => format!("data: {data}\n\n"),
        })
        .collect();
    if done {
        out.push("data: [DONE]\n\n".into());
    }
    out
}

fn openai_chunks() -> Vec<String> {
    let c = |delta: Value, finish: Value| {
        (
            None,
            json!({"id":"c1","object":"chat.completion.chunk","model":"mock","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}),
        )
    };
    sse(
        &[
            c(json!({"role":"assistant","content":"Hel"}), Value::Null),
            c(json!({"content":"lo"}), Value::Null),
            c(
                json!({"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"find","arguments":""}}]}),
                Value::Null,
            ),
            c(
                json!({"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":"}}]}),
                Value::Null,
            ),
            c(
                json!({"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"describe","arguments":"{\"node\":\"n1\"}"}}]}),
                Value::Null,
            ),
            c(
                json!({"tool_calls":[{"index":0,"function":{"arguments":"\"billing\"}"}}]}),
                Value::Null,
            ),
            c(json!({}), json!("tool_calls")),
            (
                None,
                json!({"id":"c1","object":"chat.completion.chunk","model":"mock","choices":[],
                       "usage":{"prompt_tokens":120,"completion_tokens":30,"total_tokens":150,
                                "prompt_tokens_details":{"cached_tokens":100}}}),
            ),
        ],
        true,
    )
}

fn anthropic_chunks() -> Vec<String> {
    let e = |t: &'static str, v: Value| (Some(t), v);
    sse(
        &[
            e(
                "message_start",
                json!({"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"mock","content":[],"stop_reason":null,
                   "usage":{"input_tokens":20,"cache_read_input_tokens":100,"cache_creation_input_tokens":0,"output_tokens":1}}}),
            ),
            e(
                "content_block_start",
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            ),
            e(
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}),
            ),
            e(
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}),
            ),
            e(
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
            ),
            e(
                "content_block_start",
                json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_a","name":"find","input":{}}}),
            ),
            e(
                "content_block_delta",
                json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"query\":"}}),
            ),
            e(
                "content_block_delta",
                json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"billing\"}"}}),
            ),
            e(
                "content_block_stop",
                json!({"type":"content_block_stop","index":1}),
            ),
            e(
                "content_block_start",
                json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_b","name":"describe","input":{}}}),
            ),
            e(
                "content_block_delta",
                json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"node\":\"n1\"}"}}),
            ),
            e(
                "content_block_stop",
                json!({"type":"content_block_stop","index":2}),
            ),
            e(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":30}}),
            ),
            e("message_stop", json!({"type":"message_stop"})),
        ],
        false,
    )
}

fn gemini_chunks() -> Vec<String> {
    sse(
        &[
            (
                None,
                json!({"candidates":[{"content":{"role":"model","parts":[{"text":"Hel"}]}}]}),
            ),
            (
                None,
                json!({"candidates":[{"content":{"role":"model","parts":[{"text":"lo"}]}}]}),
            ),
            (
                None,
                json!({"candidates":[{"content":{"role":"model","parts":[
                    {"functionCall":{"name":"find","args":{"query":"billing"}}},
                    {"functionCall":{"name":"describe","args":{"node":"n1"}}}]},"finishReason":"STOP"}],
                   "usageMetadata":{"promptTokenCount":120,"candidatesTokenCount":30,"totalTokenCount":150,"cachedContentTokenCount":100}}),
            ),
        ],
        false,
    )
}

fn ollama_chunks() -> Vec<String> {
    let l = |v: Value| format!("{v}\n");
    vec![
        l(
            json!({"model":"mock","created_at":"2026-11-22T00:00:00Z","message":{"role":"assistant","content":"Hel"},"done":false}),
        ),
        l(
            json!({"model":"mock","created_at":"2026-11-22T00:00:00Z","message":{"role":"assistant","content":"lo"},"done":false}),
        ),
        l(
            json!({"model":"mock","created_at":"2026-11-22T00:00:00Z","message":{"role":"assistant","content":"","tool_calls":[
            {"function":{"name":"find","arguments":{"query":"billing"}}},
            {"function":{"name":"describe","arguments":{"node":"n1"}}}]},"done":false}),
        ),
        l(
            json!({"model":"mock","created_at":"2026-11-22T00:00:00Z","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":120,"eval_count":30}),
        ),
    ]
}

/// Starts the mock on an ephemeral loopback port.
pub async fn start_mock() -> Mock {
    let reqs: Arc<Mutex<Vec<Rec>>> = Arc::default();
    let closed: Arc<AtomicBool> = Arc::default();
    let (r, c) = (reqs.clone(), closed.clone());
    let app = Router::new().fallback(move |uri: Uri, headers: HeaderMap, body: Bytes| {
        let (r, c) = (r.clone(), c.clone());
        async move {
            let path = uri.to_string();
            r.lock().unwrap().push(Rec {
                path: path.clone(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect(),
                body: serde_json::from_slice(&body).unwrap_or(Value::Null),
            });
            let (ctype, chunks) = if path.contains("streamGenerateContent") {
                ("text/event-stream", gemini_chunks())
            } else if path.contains("/anthropic/") {
                ("text/event-stream", anthropic_chunks())
            } else if path.contains("/ollama/") {
                ("application/x-ndjson", ollama_chunks())
            } else {
                ("text/event-stream", openai_chunks())
            };
            let head = stream::iter(chunks.clone()).map(|s| Ok::<_, Infallible>(Bytes::from(s)));
            let body = if path.contains("/slow/") {
                // One chunk, then hold the connection open until the client drops it.
                let guard = CloseGuard(c);
                let first = stream::iter(chunks.into_iter().take(1))
                    .map(|s| Ok::<_, Infallible>(Bytes::from(s)));
                let hold = stream::unfold(guard, |g| async move {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    Some((Ok::<_, Infallible>(Bytes::new()), g))
                });
                Body::from_stream(first.chain(hold))
            } else {
                Body::from_stream(head)
            };
            axum::response::Response::builder()
                .header("content-type", ctype)
                .body(body)
                .unwrap()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });
    Mock {
        base,
        reqs,
        closed,
        stop: Some(tx),
    }
}

// ---------------------------------------------------------------- genai checks

/// The five provider shapes G3 names: native OpenAI, Anthropic, Gemini, Ollama, and one
/// OpenAI-compatible server (LM Studio style), each at its own base URL on the mock.
pub fn providers() -> Vec<(&'static str, AdapterKind, &'static str, &'static str)> {
    vec![
        ("openai", AdapterKind::OpenAI, "openai/v1/", "o4-mini"),
        (
            "anthropic",
            AdapterKind::Anthropic,
            "anthropic/v1/",
            "claude-sonnet-4-5",
        ),
        (
            "gemini",
            AdapterKind::Gemini,
            "gemini/v1beta/",
            "gemini-2.5-flash",
        ),
        ("ollama", AdapterKind::Ollama, "ollama/", "qwen3"),
        (
            "openai-compatible",
            AdapterKind::OpenAI,
            "compat/v1/",
            "qwen3-8b",
        ),
    ]
}

/// A Client built on our own reqwest::Client: no proxy from the environment, no redirects.
pub fn our_client() -> Client {
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    Client::builder().with_reqwest(http).build().unwrap()
}

pub fn target(base: &str, kind: AdapterKind, path: &str, model: &str, key: &str) -> ServiceTarget {
    ServiceTarget {
        endpoint: Endpoint::from_owned(format!("{base}/{path}")),
        auth: AuthData::from_single(key),
        model: ModelIden::new(kind, model),
    }
}

pub fn request() -> ChatRequest {
    let tool = |name: &str, prop: &str| {
        Tool::new(name)
            .with_description(format!("{name} in the graph"))
            .with_schema(json!({"type":"object","properties":{prop:{"type":"string"}},"required":[prop],"additionalProperties":false}))
    };
    ChatRequest::new(vec![
        ChatMessage::system("You answer from the graph.").with_options(CacheControl::Ephemeral),
        ChatMessage::user("What calls billing?"),
    ])
    .with_tools(vec![tool("find", "query"), tool("describe", "node")])
}

pub fn options() -> ChatOptions {
    ChatOptions::default()
        .with_capture_usage(true)
        .with_capture_content(true)
        .with_capture_tool_calls(true)
        .with_reasoning_effort(ReasoningEffort::Low)
}

#[derive(Debug, Default)]
pub struct StreamOut {
    pub text_chunks: usize,
    pub text: String,
    pub tool_call_chunks: usize,
    pub tool_calls: Vec<(String, String, Value)>, // (call_id, name, args)
    pub usage: Option<Usage>,
}

pub async fn run_stream(
    client: &Client,
    t: ServiceTarget,
    req: ChatRequest,
) -> Result<StreamOut, String> {
    let opts = options();
    let mut res = client
        .exec_chat_stream(t, req, Some(&opts))
        .await
        .map_err(|e| e.to_string())?;
    let mut out = StreamOut::default();
    while let Some(ev) = res.stream.next().await {
        match ev.map_err(|e| e.to_string())? {
            ChatStreamEvent::Chunk(c) => {
                out.text_chunks += 1;
                out.text.push_str(&c.content);
            }
            ChatStreamEvent::ToolCallChunk(_) => out.tool_call_chunks += 1,
            ChatStreamEvent::End(end) => {
                out.usage = end.captured_usage;
                if let Some(content) = end.captured_content {
                    for p in content.into_parts() {
                        if let ContentPart::ToolCall(tc) = p {
                            out.tool_calls
                                .push((tc.call_id, tc.fn_name, tc.fn_arguments));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Opens a stream on the mock's held-open route, reads the first event, drops the stream,
/// and reports whether the mock saw the connection close within two seconds.
pub async fn cancel_by_drop(mock: &Mock) -> Result<bool, String> {
    let client = our_client();
    let t = target(
        &mock.base,
        AdapterKind::OpenAI,
        "slow/v1/",
        "o4-mini",
        "mock-key",
    );
    let mut res = client
        .exec_chat_stream(t, request(), Some(&options()))
        .await
        .map_err(|e| e.to_string())?;
    // Read until the first text chunk arrives.
    while let Some(ev) = res.stream.next().await {
        if let Ok(ChatStreamEvent::Chunk(_)) = ev {
            break;
        }
    }
    drop(res);
    for _ in 0..40 {
        if mock.closed.load(Ordering::SeqCst) {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(false)
}

/// The key header each adapter sends.
pub fn key_header(kind: AdapterKind) -> &'static str {
    match kind {
        AdapterKind::Anthropic => "x-api-key",
        AdapterKind::Gemini => "x-goog-api-key",
        _ => "authorization",
    }
}

// ---------------------------------------------------------------- keyring

/// set, get, delete, then get again (expects NoEntry) under a throwaway service name.
pub fn keyring_roundtrip() -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    let entry = keyring::Entry::new("cartograph-spike", "phase0").map_err(|e| e.to_string())?;
    log.push(format!(
        "store_status: {:?}",
        keyring::Entry::store_status()
    ));
    let secret = format!("not-a-real-key-{}", std::process::id());
    entry
        .set_password(&secret)
        .map_err(|e| format!("set: {e}"))?;
    log.push("set: ok".into());
    let got = entry.get_password().map_err(|e| format!("get: {e}"))?;
    if got != secret {
        return Err("get returned a different value".into());
    }
    log.push("get: ok (same value)".into());
    entry
        .delete_credential()
        .map_err(|e| format!("delete: {e}"))?;
    log.push("delete: ok".into());
    match entry.get_password() {
        Err(keyring::Error::NoEntry) => log.push("get after delete: NoEntry (ok)".into()),
        other => return Err(format!("get after delete: {other:?}")),
    }
    Ok(log)
}

/// One step of the upgrade check, so set and get can run from different binaries.
pub fn keyring_step(step: &str) -> Result<String, String> {
    let entry = keyring::Entry::new("cartograph-spike", "upgrade").map_err(|e| e.to_string())?;
    match step {
        "set" => entry
            .set_password("not-a-real-key")
            .map(|_| "set".into())
            .map_err(|e| e.to_string()),
        "get" => entry.get_password().map_err(|e| e.to_string()),
        "delete" => entry
            .delete_credential()
            .map(|_| "deleted".into())
            .map_err(|e| e.to_string()),
        _ => Err("step must be set, get or delete".into()),
    }
}

// ---------------------------------------------------------------- ACP

/// The client capabilities Cartograph would send in `initialize`: the default, which offers
/// no file system and no terminal.
pub fn acp_client_caps_default() -> Value {
    serde_json::to_value(agent_client_protocol::v1::schema::ClientCapabilities::default()).unwrap()
}
