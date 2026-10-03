// The AI chat's conversation: what the panel shows (`items`) and what the model sees
// (`history`, append-only). One turn runs at a time; Stop aborts the stream and the tool loop.
import Anthropic from "@anthropic-ai/sdk";
import type { BetaMessageParam, BetaTextBlockParam, BetaTool } from "@anthropic-ai/sdk/resources/beta/messages/messages";
import { create } from "zustand";
import type { EngineTransport } from "@/transport";
import { callTool, listTools } from "./agentApi";
import { runTurn, toApiTools, type TurnEnd } from "./loop";
import { useAiSettings } from "./settings";
import { overviewUpdate, systemBlocks } from "./systemPrompt";

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
  history: BetaMessageParam[];
  system: BetaTextBlockParam[] | null;
  tools: BetaTool[] | null;
  /** The overview the model last saw (to send a fresh one only when the project changed). */
  overview: string | null;
  abort: AbortController | null;
}

let convo: Conversation = fresh();
let seq = 0;
const nextId = () => `ai-${++seq}`;

function fresh(): Conversation {
  return { history: [], system: null, tools: null, overview: null, abort: null };
}

/** Builds the API client (replaced in tests). The key goes to api.anthropic.com only. */
export const clientFactory = {
  create(apiKey: string): Anthropic {
    return new Anthropic({ apiKey, dangerouslyAllowBrowser: true });
  },
};

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
  const { apiKey, model, maxIterations } = useAiSettings.getState();
  if (!body || !apiKey || isRunning()) return;
  const c = convo;
  const abort = new AbortController();
  c.abort = abort;
  useAiChat.setState({ running: true, keyRejected: false });
  push({ kind: "user", id: nextId(), text: body });

  let current: string | null = null;
  try {
    if (!c.tools) c.tools = toApiTools(await listTools(transport));
    const overview = await overviewOf(transport);
    const content: BetaTextBlockParam[] = [];
    if (!c.system) c.system = systemBlocks(overview);
    else if (overview && overview !== c.overview) content.push({ type: "text", text: overviewUpdate(overview) });
    content.push({ type: "text", text: body });
    c.overview = overview;
    c.history.push({ role: "user", content });

    const end = await runTurn({
      client: clientFactory.create(apiKey),
      model,
      system: c.system,
      tools: c.tools,
      messages: c.history,
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
      const rejected = e instanceof Anthropic.AuthenticationError || e instanceof Anthropic.PermissionDeniedError;
      push({ kind: "notice", id: nextId(), tone: "error", text: errorText(e) });
      if (rejected) useAiChat.setState({ keyRejected: true });
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

/** A user-facing message for an API failure (never includes the key). */
export function errorText(e: unknown): string {
  if (e instanceof Anthropic.AuthenticationError) return "Your API key was rejected. Check it in the AI settings.";
  if (e instanceof Anthropic.PermissionDeniedError) return "This API key isn't allowed to use this model.";
  if (e instanceof Anthropic.RateLimitError) return "Rate limited by the Anthropic API. Wait a moment and try again.";
  if (e instanceof Anthropic.APIConnectionError) return "Couldn't reach api.anthropic.com. Check your connection.";
  if (e instanceof Anthropic.InternalServerError) return "The Anthropic API had a problem. Try again.";
  if (e instanceof Anthropic.APIError) return `Request failed (${e.status ?? "error"}): ${apiMessage(e)}`;
  return e instanceof Error ? e.message : String(e);
}

function apiMessage(e: InstanceType<typeof Anthropic.APIError>): string {
  const body = e.error as { error?: { message?: string } } | undefined;
  return body?.error?.message ?? e.message;
}

/** Tests: forget everything. */
export function resetAiChat(): void {
  newChat();
  seq = 0;
}
