//! `ether-mcp`: a stdio [MCP](https://modelcontextprotocol.io) server that lets LLM clients
//! (Claude Code, Claude Desktop, ...) drive Ethereal (`agent-api`; docs/MCP.md).
//!
//! It holds no tool logic: `tools/list` and `tools/call` are proxied to the agent API of an
//! Ethereal engine (`Command::Agent { ListTools, CallTool }`, implemented by the controller),
//! so the MCP tools are always exactly the app's. The project overview is also an MCP
//! resource ([`mcp::OVERVIEW_URI`]).
//!
//! Engines ([`backend::Backend`]):
//! - **desktop** (default): the running desktop app, through its agent bridge (enabled in
//!   the app with "Allow AI agents (MCP)"; found through its runtime file);
//! - **server**: an `ether-server` (`--server ws://host:port --token T`);
//! - **headless**: an engine embedded in this process on a project file (`--project`),
//!   without audio output (the null backend); `save_project` writes the file.

pub mod backend;
pub mod cli;
pub mod headless;
pub mod mcp;
pub mod remote;
