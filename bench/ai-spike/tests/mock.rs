//! G3 and G4 checks against the in-process mock provider. No keys, no network beyond 127.0.0.1.

use ai_spike::*;

#[tokio::test]
async fn every_provider_streams_text_parallel_tools_and_usage_at_its_own_base_url() {
    let mock = start_mock().await;
    let client = our_client();
    for (name, kind, path, model) in providers() {
        let before = mock.reqs.lock().unwrap().len();
        let out = run_stream(
            &client,
            target(&mock.base, kind, path, model, "mock-key"),
            request(),
        )
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let rec = mock.reqs.lock().unwrap()[before].clone();

        assert!(
            rec.path.starts_with(&format!("/{path}")),
            "{name}: {}",
            rec.path
        );
        assert_eq!(out.text, "Hello", "{name}");
        assert!(out.text_chunks >= 2, "{name}: text was not streamed");
        let names: Vec<_> = out.tool_calls.iter().map(|t| t.1.as_str()).collect();
        assert_eq!(names, ["find", "describe"], "{name}: parallel tool calls");
        assert_eq!(out.tool_calls[0].2["query"], "billing", "{name}");
        let usage = out.usage.unwrap_or_else(|| panic!("{name}: no usage"));
        assert_eq!(usage.prompt_tokens, Some(120), "{name}");
        assert_eq!(usage.completion_tokens, Some(30), "{name}");
        if name != "ollama" {
            // Ollama's API reports no cached tokens.
            let cached = usage.prompt_tokens_details.and_then(|d| d.cached_tokens);
            assert_eq!(cached, Some(100), "{name}: cached tokens");
        }
        if kind != genai::adapter::AdapterKind::Ollama {
            assert!(
                rec.header(key_header(kind)).unwrap().contains("mock-key"),
                "{name}"
            );
        }
    }
}

#[tokio::test]
async fn reasoning_effort_and_cache_control_reach_the_wire() {
    let mock = start_mock().await;
    let client = our_client();
    for (name, kind, path, model) in providers() {
        run_stream(
            &client,
            target(&mock.base, kind, path, model, "mock-key"),
            request(),
        )
        .await
        .unwrap();
        let body = mock.reqs.lock().unwrap().last().unwrap().body.clone();
        let effort = match name {
            "anthropic" => body["thinking"]["budget_tokens"].is_number(),
            "gemini" => body["generationConfig"]["thinkingConfig"].is_object(),
            "ollama" => body["think"] == "low",
            _ => body["reasoning_effort"] == "low",
        };
        assert!(effort, "{name}: reasoning effort missing in {body}");
        if name == "anthropic" {
            assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        }
    }
}

#[tokio::test]
async fn dropping_the_stream_closes_the_connection() {
    let mock = start_mock().await;
    assert_eq!(cancel_by_drop(&mock).await, Ok(true));
}

#[test]
fn acp_default_client_capabilities_offer_no_fs_and_no_terminal() {
    let caps = acp_client_caps_default();
    assert_eq!(caps["fs"]["readTextFile"], false);
    assert_eq!(caps["fs"]["writeTextFile"], false);
    assert_eq!(caps["terminal"], false);
}

#[test]
fn keyring_set_get_delete() {
    // Linux CI without Secret Service has no default store; G4's file fallback covers that.
    if keyring::Entry::store_status().is_err() {
        eprintln!(
            "no default credential store here: {:?}",
            keyring::Entry::store_status()
        );
        return;
    }
    let log = keyring_roundtrip().unwrap();
    assert_eq!(log.last().unwrap(), "get after delete: NoEntry (ok)");
}
