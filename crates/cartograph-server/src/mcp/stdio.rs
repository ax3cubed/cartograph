//! `cartograph mcp`: the registry served over stdio on rmcp. stdout carries protocol frames only;
//! logs go to stderr and `logs/`. Each tool call runs on the blocking pool, so a slow query never
//! stalls the protocol loop, and the graph a traversal needs is built on its first use, not at
//! start-up.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam,
    ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

use super::{INSTRUCTIONS, Tools, registry, status_resource};

const STATUS_URI: &str = "cartograph://status";

/// The registry as `tools/list` gives it. Every tool but `sync` and `correct` is read-only; none
/// reaches past this machine (`openWorldHint: false`).
pub fn tools() -> Vec<Tool> {
    registry()
        .into_iter()
        .map(|t| {
            let obj = |v: Value| Arc::new(v.as_object().cloned().unwrap_or_default());
            let read_only = matches!(t.access, cartograph_api::types::Access::Read);
            Tool::new(t.wire_name(), t.description, obj(t.input))
                .with_title(t.title)
                .with_raw_output_schema(obj(t.output))
                .with_annotations(
                    ToolAnnotations::with_title(t.title)
                        .read_only(read_only)
                        .destructive(false)
                        .idempotent(t.idempotent)
                        .open_world(false),
                )
        })
        .collect()
}

#[derive(Clone)]
pub struct Handler {
    tools: Arc<Tools>,
}

impl Handler {
    pub fn new(tools: Tools) -> Self {
        Self {
            tools: Arc::new(tools),
        }
    }
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions(INSTRUCTIONS)
        .with_server_info(Implementation::new("cartograph", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|t| t.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        if !registry().iter().any(|t| t.wire_name() == name) {
            return Err(ErrorData::invalid_params("no such tool", None));
        }
        let args = request.arguments.map_or(Value::Null, Value::Object);
        let token = context.meta.get_progress_token();
        let peer = context.peer.clone();
        let rt = tokio::runtime::Handle::current();
        let tools = self.tools.clone();
        let out = tokio::task::spawn_blocking(move || {
            let notify = |done: f64, total: f64| {
                if let Some(t) = token.clone() {
                    let p = peer.clone();
                    let param = ProgressNotificationParam::new(t, done).with_total(total);
                    rt.spawn(async move {
                        let _ = p.notify_progress(param).await;
                    });
                }
            };
            tools.call(&name, args, Some(&notify))
        })
        .await
        .map_err(|_| ErrorData::internal_error("the tool stopped on an internal error", None))?;
        let text = vec![ContentBlock::text(out.content)];
        let result = match out.structured {
            Some(v) if !out.is_error => {
                let mut r = CallToolResult::structured(v);
                r.content = text;
                r
            }
            _ => CallToolResult::error(text),
        };
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut r = Resource::new(STATUS_URI, "status");
        r.description = Some("How current the index is, as the status tool says it.".into());
        r.mime_type = Some("text/markdown".into());
        Ok(ListResourcesResult::with_all_items(vec![r]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResponse, ErrorData> {
        if request.uri != STATUS_URI {
            return Err(ErrorData::resource_not_found("no such resource", None));
        }
        let tools = self.tools.clone();
        let text = tokio::task::spawn_blocking(move || status_resource(&tools))
            .await
            .map_err(|_| ErrorData::internal_error("status failed", None))?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, STATUS_URI).with_mime_type("text/markdown"),
        ])
        .into())
    }
}

/// Serves `tools` on stdin and stdout until the client closes them.
pub fn serve(tools: Tools) -> std::io::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let service = Handler::new(tools)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(std::io::Error::other)?;
        service.waiting().await.map_err(std::io::Error::other)?;
        Ok(())
    })
}
