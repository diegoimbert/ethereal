//! The MCP side: an `rmcp` server handler that proxies tools to a [`Backend`].

use std::sync::Arc;

use ether_protocol::agent::{AgentCommand, AgentToolSpec};
use ether_protocol::{Command, ReplyValue};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::backend::Backend;

/// URI of the project overview resource (same JSON as the `get_project_overview` tool).
pub const OVERVIEW_URI: &str = "ethereal://project/overview";

const SAVE_TOOL: &str = "save_project";

/// The MCP server.
#[derive(Clone)]
pub struct EtherMcp {
    backend: Arc<dyn Backend>,
}

impl EtherMcp {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }

    async fn request(&self, command: Command) -> Result<ReplyValue, String> {
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || backend.request(command))
            .await
            .map_err(|e| e.to_string())?
    }

    /// The engine's tools, or the built-in registry while it is unreachable (so clients
    /// still see the tools; calls then explain how to connect).
    async fn tool_specs(&self) -> Vec<AgentToolSpec> {
        match self.request(Command::Agent(AgentCommand::ListTools)).await {
            Ok(ReplyValue::AgentTools { tools }) => tools,
            Ok(other) => {
                tracing::warn!(?other, "unexpected reply to ListTools");
                ether_controller::agent::tool_specs()
            }
            Err(e) => {
                tracing::warn!(%e, "ListTools failed; listing the built-in registry");
                ether_controller::agent::tool_specs()
            }
        }
    }

    /// Run a tool: `(content, is_error)`.
    pub async fn call(&self, name: &str, input: &JsonObject) -> (String, bool) {
        let command = Command::Agent(AgentCommand::CallTool {
            name: name.to_string(),
            input: serde_json::Value::Object(input.clone()).to_string(),
        });
        let (mut content, is_error) = match self.request(command).await {
            Ok(ReplyValue::AgentToolResult { content, is_error }) => (content, is_error),
            Ok(other) => (format!("unexpected reply: {other:?}"), true),
            Err(e) => (e, true),
        };
        if name == SAVE_TOOL && !is_error {
            let backend = self.backend.clone();
            match tokio::task::spawn_blocking(move || backend.after_save()).await {
                Ok(Ok(Some(note))) => content = with_note(&content, &note),
                Ok(Ok(None)) => {}
                Ok(Err(e)) => return (e, true),
                Err(e) => return (e.to_string(), true),
            }
        }
        (content, is_error)
    }
}

/// Add `"note"` to a JSON object result (or append to plain text).
fn with_note(content: &str, note: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(serde_json::Value::Object(mut o)) => {
            o.insert("file".into(), note.into());
            serde_json::Value::Object(o).to_string()
        }
        _ => format!("{content}\n{note}"),
    }
}

fn to_tool(spec: AgentToolSpec) -> Tool {
    let schema: JsonObject = serde_json::from_str(&spec.input_schema).unwrap_or_else(|_| {
        let mut o = JsonObject::new();
        o.insert("type".into(), "object".into());
        o
    });
    Tool::new(spec.name, spec.description, Arc::new(schema))
}

const INSTRUCTIONS: &str = "Ethereal is a DAW (digital audio workstation). These tools edit the open \
project the user also sees. Start with get_project_overview to learn the ids. Positions and lengths \
are in beats (quarter notes): arrangement positions from the song start, note positions from the \
clip start (in 4/4 a bar is 4 beats). Volume is in dB, pan -1..1, MIDI pitch 0-127 (60 = C3), \
velocity 1-127. Every editing call is one undo step the user can undo.";

impl ServerHandler for EtherMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(
            Implementation::new("ethereal", env!("CARGO_PKG_VERSION")).with_title("Ethereal"),
        )
        .with_instructions(format!(
            "{INSTRUCTIONS} Connected to {}.",
            self.backend.describe()
        ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self.tool_specs().await.into_iter().map(to_tool).collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let input = request.arguments.unwrap_or_default();
        let (content, is_error) = self.call(&request.name, &input).await;
        let result = if is_error {
            CallToolResult::error(vec![ContentBlock::text(content)])
        } else {
            CallToolResult::success(vec![ContentBlock::text(content)])
        };
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut overview = Resource::new(OVERVIEW_URI, "project-overview")
            .with_title("Ethereal project overview")
            .with_description(
                "The open project: tempo, time signature, tracks with their devices and clips (ids, beats), selection, transport. Same JSON as the get_project_overview tool.",
            );
        overview.mime_type = Some("application/json".into());
        Ok(ListResourcesResult::with_all_items(vec![overview]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        if request.uri != OVERVIEW_URI {
            return Err(McpError::resource_not_found(
                format!("no resource {}", request.uri),
                None,
            ));
        }
        let (content, is_error) = self.call("get_project_overview", &JsonObject::new()).await;
        if is_error {
            return Err(McpError::internal_error(content, None));
        }
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(content, OVERVIEW_URI).with_mime_type("application/json"),
        ])
        .into())
    }
}
