// ai-chat: in-app AI chat that edits the project through the agent API (Command::Agent),
// the same tools the MCP server exposes. Mounted as the "Ask AI" left rail tab.
export { AiChatPanel } from "./AiChatPanel";
export { AI_TAB, isAiShortcut, openAiChat, useAiChatShortcut } from "./open";
