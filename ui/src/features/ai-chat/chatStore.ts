// The AI chat's conversation: what the panel shows (`items`) and what the model sees (the
// provider session's history, append-only). One turn runs at a time; Stop aborts the stream
// and the tool loop.
import { create } from "zustand";
import type { EngineTransport } from "@/transport";
import { callTool, listTools } from "./agentApi";
import { runTurn, type TurnEnd } from "./loop";
import { createSession, ProviderError, PROVIDERS, toolDefs, type ChatSession, type ProviderId, type ToolDef } from "./providers";
import { activeConnection, missingSetup, useAiSettings } from "./settings";
import { overviewUpdate, systemBlocks } from "./systemPrompt";

export { clientFactory } from "./providers/anthropic";

export type ChatItem =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string }
  | { kind: "tool"; id: string; name: string; input: unknown; result: { content: string; is_error: boolean } | null }
  | { kind: "notice"; id: string; tone: "info" | "error"; text: string };

export interface AiChatState {
  items: ChatItem[];
  running: boolean;
  /** The key was rejected: the panel shows the key setup again. */
  keyRejected: boolean;
}

export const useAiChat = create<AiChatState>()(() => ({ items: [], running: false, keyRejected: false }));

/** Model-side state of the current conversation (not rendered). */
interface Conversation {
  /** The model-side conversation, and the provider it was started with. */
  session: ChatSession | null;
  provider: ProviderId | null;
  tools: ToolDef[] | null;
  /** The overview the model last saw (to send a fresh one only when the project changed). */
  overview: string | null;
  abort: AbortController | null;
}

let convo: Conversation = fresh();
let seq = 0;
const nextId = () => `ai-${++seq}`;

function fresh(): Conversation {
  return { session: null, provider: null, tools: null, overview: null, abort: null };
}

function push(item: ChatItem): void {
  useAiChat.setState((s) => ({ items: [...s.items, item] }));
}

function patchItem(id: string, f: (item: ChatItem) => ChatItem): void {
  useAiChat.setState((s) => ({ items: s.items.map((it) => (it.id === id ? f(it) : it)) }));
}

async function overviewOf(transport: EngineTransport): Promise<string | null> {
  const r = await callTool(transport, "get_project_overview", {});
  return r.is_error ? null : r.content;
}

/** Start a new conversation (Stop first if a turn is running). */
export function newChat(): void {
  convo.abort?.abort();
  convo = fresh();
  useAiChat.setState({ items: [], running: false, keyRejected: false });
}

/** Stop the running turn: aborts the stream; no further tool runs. */
export function stopTurn(): void {
  convo.abort?.abort();
}

export function isRunning(): boolean {
  return useAiChat.getState().running;
}

/** Send a user message and run the turn to its end. */
export async function sendMessage(transport: EngineTransport, text: string): Promise<void> {
  const body = text.trim();
  const settings = useAiSettings.getState();
  const { provider, maxIterations } = settings;
  if (!body || missingSetup(settings) || isRunning()) return;
  const conn = activeConnection(settings);
  const c = convo;
  const abort = new AbortController();
  c.abort = abort;
  useAiChat.setState({ running: true, keyRejected: false });
  push({ kind: "user", id: nextId(), text: body });

  let current: string | null = null;
  try {
    if (!c.tools) c.tools = toolDefs(await listTools(transport));
    const overview = await overviewOf(transport);
    const parts: string[] = [];
    if (c.session && c.provider !== provider) {
      // Histories don't carry over between wire formats: the new provider starts fresh.
      push({ kind: "notice", id: nextId(), tone: "info", text: `Switched to ${PROVIDERS[provider].label}: it starts fresh and doesn't see the messages above.` });
      c.session = null;
    }
    if (!c.session) {
      c.session = createSession(provider, systemBlocks(overview), c.tools);
      c.provider = provider;
    } else if (overview && overview !== c.overview) parts.push(overviewUpdate(overview));
    parts.push(body);
    c.overview = overview;
    c.session.addUser(parts);

    const end = await runTurn({
      session: c.session,
      conn,
      maxIterations,
      signal: abort.signal,
      callTool: (name, input) => callTool(transport, name, input),
      onEvent: (e) => {
        if (convo !== c) return;
        switch (e.type) {
          case "text_start":
            current = null;
            break;
          case "text": {
            if (current === null) {
              current = nextId();
              push({ kind: "assistant", id: current, text: e.delta });
            } else {
              const id: string = current;
              patchItem(id, (it) => (it.kind === "assistant" ? { ...it, text: it.text + e.delta } : it));
            }
            break;
          }
          case "tool_call":
            current = null;
            push({ kind: "tool", id: e.id, name: e.name, input: e.input, result: null });
            break;
          case "tool_result":
            patchItem(e.id, (it) => (it.kind === "tool" ? { ...it, result: { content: e.content, is_error: e.is_error } } : it));
            break;
        }
      },
    });
    if (convo === c) {
      const notice = endNotice(end);
      if (notice) push({ kind: "notice", id: nextId(), ...notice });
      // What the model now knows of the project (its own edits included).
      c.overview = await overviewOf(transport);
    }
  } catch (e) {
    if (convo === c) {
      push({ kind: "notice", id: nextId(), tone: "error", text: errorText(e) });
      if (e instanceof ProviderError && e.keyRejected) useAiChat.setState({ keyRejected: true });
    }
  } finally {
    if (convo === c) {
      c.abort = null;
      useAiChat.setState({ running: false });
    }
  }
}

function endNotice(end: TurnEnd): { tone: "info" | "error"; text: string } | null {
  switch (end.reason) {
    case "end_turn":
      return null;
    case "stopped":
      return { tone: "info", text: "Stopped." };
    case "capped":
      return { tone: "info", text: `Paused after ${end.iterations} tool rounds (the per-turn limit in settings). Say "continue" to go on.` };
    case "refusal":
      return { tone: "error", text: "The model declined this request." };
    case "max_tokens":
      return { tone: "error", text: "The response was cut off (output limit). Try a smaller request." };
  }
}

/** A user-facing message for an API failure (adapters throw `ProviderError`s; never includes the key). */
export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Tests: forget everything. */
export function resetAiChat(): void {
  newChat();
  seq = 0;
}
