//! Agent API (`agent-api`, owner request; docs/MCP.md, CONTRACTS.md §13): a tool registry
//! for LLM agents, implemented by the controller (`ether-controller/src/agent`), so it works
//! the same on the desktop app, the remote engine (`ether-server`) and the web build.
//!
//! Both the in-app AI chat (`ai-chat`) and the MCP server (`ether-mcp`) drive the app only
//! through these two commands:
//! - `ListTools` replies `ReplyValue::AgentTools` with every tool (name, LLM-facing
//!   description, JSON Schema of its input).
//! - `CallTool` replies `ReplyValue::AgentToolResult`. A tool that edits the document is ONE
//!   undo step, replicated in a collab session, and its `Event::Patch`es arrive before the
//!   reply (like any command). An unknown tool or an invalid input is a result with
//!   `is_error: true` (the model must see it), never a `CommandError`.
//!
//! JSON values are carried as JSON *text* (`input_schema`, `input`, `content`) to keep the
//! generated TypeScript simple; both sides parse it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::Beats;
use crate::model::ids::{ClipId, DeviceId, NoteId, TrackId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AgentCommand {
    /// Replies `AgentTools`.
    ListTools,
    /// Run tool `name` with `input` (a JSON object, as text). Replies `AgentToolResult`.
    CallTool { name: String, input: String },
    /// What the user has selected in the UI: the tools read it ("this clip", ties between
    /// candidates). Sent when the selection changes. Replies `Unit`.
    SetSelection { selection: AgentSelection },
}

/// The UI's selection, as the agent tools see it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct AgentSelection {
    /// The edit (insert) cursor on the timeline.
    #[serde(default)]
    pub cursor: Option<Beats>,
    #[serde(default)]
    pub tracks: Vec<TrackId>,
    #[serde(default)]
    pub clips: Vec<ClipId>,
    #[serde(default)]
    pub notes: Vec<NoteId>,
    #[serde(default)]
    pub devices: Vec<DeviceId>,
    /// The clip open in the piano roll.
    #[serde(default)]
    pub editing_clip: Option<ClipId>,
}

/// One tool of the registry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AgentToolSpec {
    /// snake_case, unique.
    pub name: String,
    /// What the tool does, written for an LLM (units, id sources, limits).
    pub description: String,
    /// JSON Schema (an `object` schema) of the input, as JSON text.
    pub input_schema: String,
}
