// The AI chat's conversation: what the panel shows (`items`) and what the model sees (the
// provider session's history, append-only). One turn runs at a time; Stop aborts the stream
// and the tool loop. The conversation belongs to the open project: kept on this machine
// (`persist.ts`) after every turn, restored when the project opens again (a reload too);
// "clear chat" (`newChat`) forgets it. Nothing is sent before the user agreed, once per
// provider, to send it the project's contents (`hasConsent`).
import { create } from "zustand";
import { t } from "@/i18n";
import { useProjectStore } from "@/state";
import { clearChat, loadChat, saveChat, type SavedItem } from "./persist";
import type { EngineTransport } from "@/transport";
import { callTool, listTools } from "./agentApi";
import { runTurn, type TurnEnd } from "./loop";
import { createSession, ProviderError, providerName, toolDefs, type ChatSession, type ProviderId, type ToolDef } from "./providers";
import { activeConnection, hasConsent, missingSetup, useAiSettings } from "./settings";
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
  /** A saved model-side history, restored into the session the next message creates. */
  restored: { provider: ProviderId; history: unknown[] } | null;
}

let convo: Conversation = fresh();
let seq = 0;
const nextId = () => `ai-${++seq}`;
/** The project the conversation belongs to. */
let projectId: string | null = null;

function fresh(): Conversation {
  return { session: null, provider: null, tools: null, overview: null, abort: null, restored: null };
}

/** Keep the conversation of the open project. */
function save(): void {
  if (!projectId) return;
  const { items } = useAiChat.getState();
  const history = convo.session ? [...convo.session.history] : (convo.restored?.history ?? null);
  const provider = convo.session ? convo.provider : (convo.restored?.provider ?? null);
  saveChat(projectId, { items: items as unknown as SavedItem[], provider: history ? provider : null, history });
}

/** The open project changed: keep the old conversation, show the new one's. */
function switchProject(next: string | null): void {
  if (next === projectId) return;
  if (convo.abort) convo.abort.abort();
  save();
  projectId = next;
  convo = fresh();
  const saved = next ? loadChat(next) : null;
  const items = (saved?.items ?? []) as unknown as ChatItem[];
  for (const it of items) {
    const n = /^ai-(\d+)$/.exec(it.id);
    if (n) seq = Math.max(seq, Number(n[1]));
  }
  if (saved?.provider && saved.history) convo.restored = { provider: saved.provider, history: saved.history };
  useAiChat.setState({ items, running: false, keyRejected: false });
}

useProjectStore.subscribe((s) => switchProject(s.project?.id ?? null));

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

/** Clear the conversation and forget the saved one (Stop first if a turn is running). */
export function newChat(): void {
  convo.abort?.abort();
  convo = fresh();
  useAiChat.setState({ items: [], running: false, keyRejected: false });
  if (projectId) clearChat(projectId);
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
  if (!body || missingSetup(settings) || !hasConsent(settings) || isRunning()) return;
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
    if ((c.session && c.provider !== provider) || (c.restored && c.restored.provider !== provider)) {
      // Histories don't carry over between wire formats: the new provider starts fresh.
      push({
        kind: "notice",
        id: nextId(),
        tone: "info",
        text: t("ai_chat.notice.switched", { provider: providerName(provider) }),
      });
      c.session = null;
      c.restored = null;
    }
    if (!c.session) {
      c.session = createSession(provider, systemBlocks(overview), c.tools);
      c.provider = provider;
      // A conversation kept from before (a reload): the model sees it again, and the project
      // as it is now.
      if (c.restored) {
        c.session.restore(c.restored.history);
        c.restored = null;
        if (overview) parts.push(overviewUpdate(overview));
      }
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
      save();
    }
  }
}

function endNotice(end: TurnEnd): { tone: "info" | "error"; text: string } | null {
  switch (end.reason) {
    case "end_turn":
      return null;
    case "stopped":
      return { tone: "info", text: t("ai_chat.notice.stopped") };
    case "capped":
      return { tone: "info", text: t("ai_chat.notice.paused", { count: end.iterations }) };
    case "refusal":
      return { tone: "error", text: t("ai_chat.notice.declined") };
    case "max_tokens":
      return { tone: "error", text: t("ai_chat.notice.output_limit") };
  }
}

/** A user-facing message for an API failure (adapters throw `ProviderError`s; never includes the key). */
export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Tests: forget everything (the saved conversations stay in localStorage). */
export function resetAiChat(): void {
  convo.abort?.abort();
  convo = fresh();
  useAiChat.setState({ items: [], running: false, keyRejected: false });
  projectId = null;
  seq = 0;
}
