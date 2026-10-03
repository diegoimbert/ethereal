//! Agent API (`agent-api`; docs/MCP.md, CONTRACTS.md §13): the LLM tool registry behind
//! `Command::Agent`, shared by the in-app AI chat and the MCP server (`ether-mcp`).
//!
//! - `ListTools` → [`tools::TOOLS`] (name, description, JSON Schema).
//! - `CallTool` → validate the input against the tool's schema ([`schema`]), run the tool,
//!   reply `AgentToolResult { content, is_error }`. Unknown tools, invalid inputs and
//!   rejected edits are `is_error` results (never a `CommandError`), so the model sees them.
//! - A tool runs as ordinary document commands: edits go through `edit_with` as ONE undo
//!   step labelled `AI: <action>` (atomic, collab-replicated, patches emitted before the
//!   reply); multi-command tools that need non-document commands (media import) share one
//!   internal gesture.
//! - Read tools are compact JSON; long lists are paginated (`offset`/`limit`) or truncated
//!   with a `note` that says how to get the rest.
//!
//! Runtime state ([`AgentState`]): the last selection the UI shared (`Collab::SetPresence`,
//! for `get_project_overview`) and the status of recent export jobs (observed from the
//! controller's own `Event::Export`s through [`ExportTap`]).

mod edit;
mod read;
pub(crate) mod schema;
pub(crate) mod tools;

use ether_core::protocol::agent::{AgentCommand, AgentToolSpec};
use ether_core::protocol::collab::{CollabCommand, PresenceState};
use ether_core::protocol::export::{ExportEvent, ExportResult};
use ether_core::protocol::model::GestureId;
use ether_core::protocol::{
    ClientMessage, Command, CommandError, ErrorCode, Event, ReplyValue, ServerMessage,
};
use serde_json::{Value, json};

use crate::store::{Library, ProjectStore};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices, MessageSink, doc};

use schema::{Args, InputError};

/// Export jobs remembered for `get_export_status`.
const EXPORTS_KEPT: usize = 8;

/// Why a tool call failed (becomes an `is_error` result).
#[derive(Debug)]
pub(crate) enum ToolError {
    Input(InputError),
    Command(CommandError),
}

impl From<InputError> for ToolError {
    fn from(e: InputError) -> Self {
        Self::Input(e)
    }
}

impl From<CommandError> for ToolError {
    fn from(e: CommandError) -> Self {
        Self::Command(e)
    }
}

impl ToolError {
    pub fn input(message: impl Into<String>) -> Self {
        Self::Input(InputError::new(message))
    }

    fn message(&self) -> String {
        match self {
            Self::Input(e) => format!("Invalid input: {}", e.0),
            Self::Command(e) => {
                let kind = match e.code {
                    ErrorCode::NotFound => "Not found",
                    ErrorCode::InvalidArgument => "Rejected",
                    ErrorCode::Unsupported => "Not supported here",
                    ErrorCode::InvalidState => "Not possible now",
                    _ => "Failed",
                };
                format!("{kind}: {}", e.message)
            }
        }
    }
}

pub(crate) type ToolResult = Result<Value, ToolError>;

#[derive(Clone, Debug, PartialEq)]
enum ExportStatus {
    Running { progress: f32 },
    Done { result: ExportResult },
    Failed { message: String },
    Cancelled,
}

/// Agent runtime state (not in the document).
#[derive(Default)]
pub(crate) struct AgentState {
    /// The last presence the UI sent (its selection), even outside a collab session.
    selection: Option<PresenceState>,
    /// Recent export jobs, oldest first.
    exports: Vec<(String, ExportStatus)>,
}

impl AgentState {
    fn export_mut(&mut self, job: &str) -> &mut ExportStatus {
        if let Some(i) = self.exports.iter().position(|(j, _)| j == job) {
            return &mut self.exports[i].1;
        }
        if self.exports.len() >= EXPORTS_KEPT {
            self.exports.remove(0);
        }
        self.exports
            .push((job.to_string(), ExportStatus::Running { progress: 0.0 }));
        &mut self.exports.last_mut().expect("just pushed").1
    }

    fn absorb(&mut self, events: Vec<ExportEvent>) {
        for e in events {
            match e {
                ExportEvent::Progress { job, progress } => {
                    *self.export_mut(&job) = ExportStatus::Running { progress }
                }
                ExportEvent::Done { job, result } => {
                    *self.export_mut(&job) = ExportStatus::Done { result }
                }
                ExportEvent::Failed { job, message } => {
                    *self.export_mut(&job) = ExportStatus::Failed { message }
                }
                ExportEvent::Cancelled { job } => *self.export_mut(&job) = ExportStatus::Cancelled,
            }
        }
    }
}

/// Forwards every message and keeps a copy of the export events (for
/// `get_export_status`). Wraps the sink of `handle`/`tick`.
pub(crate) struct ExportTap<'a> {
    inner: &'a mut dyn MessageSink,
    exports: Vec<ExportEvent>,
}

impl<'a> ExportTap<'a> {
    pub fn new(inner: &'a mut dyn MessageSink) -> Self {
        Self {
            inner,
            exports: Vec::new(),
        }
    }
}

impl MessageSink for ExportTap<'_> {
    fn send(&mut self, message: ServerMessage) {
        if let ServerMessage::Event(Event::Export { event }) = &message {
            self.exports.push(event.clone());
        }
        self.inner.send(message);
    }
}

/// The registry as `AgentToolSpec`s.
pub fn tool_specs() -> Vec<AgentToolSpec> {
    tools::TOOLS
        .iter()
        .map(|t| AgentToolSpec {
            name: t.name.to_string(),
            description: t.description.to_string(),
            input_schema: (t.schema)().to_string(),
        })
        .collect()
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Record what the agent tools need from other commands (the UI's selection).
    pub(crate) fn agent_observe(&mut self, command: &Command) {
        if let Command::Collab(CollabCommand::SetPresence { presence }) = command {
            self.agent.selection = Some(presence.clone());
        }
    }

    /// Keep the export events a `handle`/`tick` emitted.
    pub(crate) fn agent_absorb(&mut self, tap: ExportTap<'_>) {
        if !tap.exports.is_empty() {
            self.agent.absorb(tap.exports);
        }
    }

    pub(crate) fn agent_command(
        &mut self,
        c: &AgentCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            AgentCommand::ListTools => Ok(ReplyValue::AgentTools {
                tools: tool_specs(),
            }),
            AgentCommand::CallTool { name, input } => {
                let (content, is_error) = match self.call_tool(name, input, now, out) {
                    Ok(v) => (
                        match v {
                            Value::String(s) => s,
                            v => v.to_string(),
                        },
                        false,
                    ),
                    Err(e) => (e.message(), true),
                };
                Ok(ReplyValue::AgentToolResult { content, is_error })
            }
        }
    }

    fn call_tool(
        &mut self,
        name: &str,
        input: &str,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let Some(def) = tools::find(name) else {
            let names: Vec<&str> = tools::TOOLS.iter().map(|t| t.name).collect();
            return Err(ToolError::input(format!(
                "unknown tool `{name}`. Available tools: {}",
                names.join(", ")
            )));
        };
        let input: Value = if input.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(input)
                .map_err(|e| ToolError::input(format!("not valid JSON: {e}")))?
        };
        schema::validate(&(def.schema)(), &input, "input")?;
        let Some(map) = input.as_object() else {
            return Err(ToolError::input("expected a JSON object"));
        };
        let a = Args(map);
        match def.name {
            // Reading.
            "get_project_overview" => self.tool_overview(),
            "get_track" => self.tool_get_track(a),
            "get_clip_notes" => self.tool_get_clip_notes(a),
            "list_device_types" => self.tool_list_device_types(a),
            "get_device_params" => self.tool_get_device_params(a),
            "get_export_status" => self.tool_export_status(a),
            "search_browser" => self.tool_search_browser(a, now, out),
            // Editing.
            "create_track" => self.tool_create_track(a, now, out),
            "delete_track" => self.tool_delete_track(a, now, out),
            "rename_track" => self.tool_rename_track(a, now, out),
            "set_track_mix" => self.tool_set_track_mix(a, now, out),
            "add_device" => self.tool_add_device(a, now, out),
            "remove_device" => self.tool_remove_device(a, now, out),
            "set_device_param" => self.tool_set_device_param(a, now, out),
            "create_midi_clip" => self.tool_create_midi_clip(a, now, out),
            "add_notes" => self.tool_add_notes(a, now, out),
            "remove_notes" => self.tool_remove_notes(a, now, out),
            "set_clip" => self.tool_set_clip(a, now, out),
            "delete_clip" => self.tool_delete_clip(a, now, out),
            "duplicate_clip" => self.tool_duplicate_clip(a, now, out),
            "set_tempo" => self.tool_set_tempo(a, now, out),
            "set_time_signature" => self.tool_set_time_signature(a, now, out),
            "transport" => self.tool_transport(a, now, out),
            "undo" => self.tool_undo_redo(true, now, out),
            "redo" => self.tool_undo_redo(false, now, out),
            "save_project" => self.tool_save(now, out),
            "load_browser_item" => self.tool_load_browser_item(a, now, out),
            "export_audio" => self.tool_export(a, now, out),
            other => Err(ToolError::Command(crate::tx::internal(format!(
                "tool `{other}` has no handler"
            )))),
        }
    }

    // ─── Execution helpers ──────────────────────────────────────────────────────────────

    fn agent_project(&self) -> Result<&ether_core::protocol::model::Project, ToolError> {
        self.doc
            .as_ref()
            .map(|d| &d.project)
            .ok_or_else(|| ToolError::Command(crate::handlers::no_project()))
    }

    fn agent_id<I: ether_core::protocol::model::Id>(&mut self, now: u64) -> I {
        self.ids.next(now)
    }

    /// Apply document commands as ONE atomic undo step labelled `label` (like
    /// `Edit::Batch`, but transport-domain document commands are allowed too).
    fn agent_batch(
        &mut self,
        label: &str,
        commands: &[Command],
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Result<(), ToolError> {
        let doc = self
            .doc
            .as_ref()
            .ok_or_else(crate::handlers::no_project)?;
        let current = Some(doc.project.id);
        for c in commands {
            if !doc::is_document_command(c, current) {
                return Err(ToolError::Command(crate::tx::internal(format!(
                    "{} is not a document command",
                    doc::label_of(c)
                ))));
            }
            crate::freeze::check_editable(&doc.project, c)?;
        }
        self.edit_with(label, None, now, out, |ctx| {
            for c in commands {
                doc::apply(ctx, c)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Run one command through the normal dispatch (non-document commands: transport,
    /// undo, export, ...), optionally inside `gesture`.
    fn agent_dispatch(
        &mut self,
        command: Command,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Result<ReplyValue, ToolError> {
        let msg = ClientMessage {
            id: 0,
            gesture,
            command,
        };
        Ok(self.dispatch(&msg, now, out)?)
    }

    /// Run commands in order inside one internal gesture (one undo step for the edits
    /// among them). Stops at the first error (earlier edits stay, in that step).
    fn agent_steps(
        &mut self,
        commands: Vec<Command>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Result<Vec<ReplyValue>, ToolError> {
        let gesture = self.new_gesture();
        let mut replies = Vec::with_capacity(commands.len());
        let mut result = Ok(());
        for c in commands {
            match self.agent_dispatch(c, Some(gesture), now, out) {
                Ok(v) => replies.push(v),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        if let Some(doc) = self.doc.as_mut() {
            doc.history.end_gesture(gesture);
        }
        result.map(|()| replies)
    }

    /// Undo/redo availability for results.
    fn agent_history(&self) -> Value {
        match self.doc.as_ref() {
            Some(d) => {
                let h = d.history.state();
                json!({
                    "can_undo": h.can_undo,
                    "undo": h.undo_label,
                    "can_redo": h.can_redo,
                    "redo": h.redo_label,
                })
            }
            None => Value::Null,
        }
    }
}

/// Undo label of a tool edit.
pub(crate) fn label(action: &str) -> String {
    format!("AI: {action}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_well_formed() {
        let mut names = std::collections::BTreeSet::new();
        for t in tools::TOOLS {
            assert!(names.insert(t.name), "duplicate tool {}", t.name);
            assert!(
                t.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
                "{} is not snake_case",
                t.name
            );
            assert!(t.description.len() > 20, "{} needs a description", t.name);
            let s = (t.schema)();
            assert_eq!(s["type"], "object", "{}", t.name);
            assert_eq!(s["additionalProperties"], false, "{}", t.name);
            for r in s["required"].as_array().unwrap() {
                assert!(
                    s["properties"].get(r.as_str().unwrap()).is_some(),
                    "{}: required {r} is not a property",
                    t.name
                );
            }
            for (k, p) in s["properties"].as_object().unwrap() {
                assert!(
                    p.get("type").is_some(),
                    "{}.{k} has no type (schemas must be explicit)",
                    t.name
                );
            }
        }
    }
}
