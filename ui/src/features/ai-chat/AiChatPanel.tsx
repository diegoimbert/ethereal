// The AI chat (left rail tab "Ask AI"): talk to a model (Anthropic, or any OpenAI-compatible
// provider), which edits the open project through the agent API (the same tools as the MCP
// server). BYOK: the user's own API key, or a local server.
import "./aiChat.css";
import clsx from "clsx";
import { ArrowUp, Check, CircleAlert, LoaderCircle, Settings, Square, SquarePen, Wrench } from "lucide-react";
import { useContext, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { Button, IconButton, MOD_KEY, Select } from "@/kit";
import { TransportContext } from "@/transport";
import { AiSettingsView } from "./AiSettingsView";
import { KeySetup } from "./KeySetup";
import { newChat, sendMessage, stopTurn, useAiChat, type ChatItem } from "./chatStore";
import { useAiFocus } from "./open";
import { PROVIDERS } from "./providers";
import { missingSetup, modelLabel, useAiSettings } from "./settings";
import { pretty, summarizeInput, summarizeResult } from "./toolSummary";

const EXAMPLES = [
  "Create a track called Bass and add a C minor arpeggio",
  "Set the tempo to 92 BPM",
  "Write a 4-bar chord progression in A minor on a new MIDI track",
];

export function AiChatPanel() {
  const missing = useAiSettings((s) => missingSetup(s));
  const providerLabel = useAiSettings((s) => PROVIDERS[s.provider].label);
  const keyRejected = useAiChat((s) => s.keyRejected);
  const [settings, setSettings] = useState(false);
  return (
    <div className="eth-ai" data-testid="ai-chat-panel">
      <Toolbar settings={settings} onSettings={() => setSettings((v) => !v)} />
      {settings ? (
        <AiSettingsView onDone={() => setSettings(false)} />
      ) : missing === "key" ? (
        <KeySetup />
      ) : missing === "model" ? (
        <div className="eth-ai__setup">
          <p className="eth-ai__setup-title">Choose a model</p>
          <p className="eth-ai__hint">Enter the model to use with {providerLabel} in the AI settings.</p>
          <Button size="sm" tone="accent" onClick={() => setSettings(true)}>
            Open AI settings
          </Button>
        </div>
      ) : (
        <>
          {keyRejected && <KeySetup rejected />}
          <Messages />
          <Composer />
        </>
      )}
    </div>
  );
}

function Toolbar({ settings, onSettings }: { settings: boolean; onSettings(): void }) {
  const provider = useAiSettings((s) => s.provider);
  const model = useAiSettings((s) => s.model);
  const setModel = useAiSettings((s) => s.setModel);
  const empty = useAiChat((s) => s.items.length === 0);
  const p = PROVIDERS[provider];
  // The provider's suggestions, plus a model typed in the settings.
  const models = model && !p.models.includes(model) ? [model, ...p.models] : [...p.models];
  return (
    <div className="eth-ai__toolbar">
      <Select<string>
        size="sm"
        aria-label="Model"
        className="eth-ai__model"
        options={models.map((m) => ({ value: m, label: modelLabel(m), group: p.label }))}
        value={model}
        placeholder="Choose a model"
        onChange={setModel}
        data-testid="ai-model"
      />
      <span className="eth-ai__spacer" />
      <IconButton size="sm" tone="ghost" label="New chat" icon={<SquarePen />} disabled={empty} onClick={newChat} />
      <IconButton size="sm" tone="ghost" label="AI settings" icon={<Settings />} active={settings} onClick={onSettings} />
    </div>
  );
}

function Messages() {
  const items = useAiChat((s) => s.items);
  const running = useAiChat((s) => s.running);
  const ref = useRef<HTMLOListElement>(null);
  const stick = useRef(true);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [items]);
  if (items.length === 0) return <EmptyState />;
  const last = items.at(-1);
  return (
    <ol
      ref={ref}
      className="eth-ai__list"
      aria-label="Conversation"
      aria-live="polite"
      data-testid="ai-messages"
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 8;
      }}
    >
      {items.map((it) => (
        <Item key={it.id} item={it} />
      ))}
      {running && last?.kind !== "assistant" && (
        <li className="eth-ai__thinking" data-testid="ai-working">
          <LoaderCircle className="eth-ai__spin" aria-hidden /> Working…
        </li>
      )}
    </ol>
  );
}

function EmptyState() {
  const transport = useContext(TransportContext)?.transport ?? null;
  return (
    <div className="eth-ai__empty">
      <p className="eth-ai__hint">Ask for an edit, or try:</p>
      {EXAMPLES.map((ex) => (
        <button key={ex} type="button" className="eth-ai__example" disabled={!transport} onClick={() => transport && void sendMessage(transport, ex)}>
          {ex}
        </button>
      ))}
    </div>
  );
}

function Item({ item }: { item: ChatItem }) {
  switch (item.kind) {
    case "user":
      return (
        <li className="eth-ai__msg eth-ai__msg--user" data-testid="ai-message" data-role="user">
          <p className="eth-ai__text">{item.text}</p>
        </li>
      );
    case "assistant":
      return (
        <li className="eth-ai__msg eth-ai__msg--assistant" data-testid="ai-message" data-role="assistant">
          <p className="eth-ai__text">{item.text}</p>
        </li>
      );
    case "tool":
      return (
        <li className="eth-ai__tool-row">
          <ToolChip item={item} />
        </li>
      );
    case "notice":
      return (
        <li className={clsx("eth-ai__notice", item.tone === "error" && "eth-ai__notice--error")} role={item.tone === "error" ? "alert" : undefined}>
          {item.text}
        </li>
      );
  }
}

/** A tool call: name, short input, then its result (collapsed; click for the details). */
function ToolChip({ item }: { item: Extract<ChatItem, { kind: "tool" }> }) {
  const state = item.result === null ? "running" : item.result.is_error ? "error" : "ok";
  const icon = state === "running" ? <LoaderCircle className="eth-ai__spin" /> : state === "error" ? <CircleAlert /> : <Check />;
  return (
    <details className={clsx("eth-ai__tool", `eth-ai__tool--${state}`)} data-testid="ai-tool" data-tool={item.name} data-state={state}>
      <summary className="eth-ai__tool-summary">
        <span className="eth-ai__tool-icon" aria-hidden>
          {icon}
        </span>
        <span className="eth-ai__tool-name">{item.name}</span>
        <span className="eth-ai__tool-input">{summarizeInput(item.input)}</span>
        {item.result && <span className="eth-ai__tool-result">{item.result.is_error ? summarizeResult(item.result.content) : ""}</span>}
      </summary>
      <div className="eth-ai__tool-details">
        <span className="eth-ai__tool-label">
          <Wrench aria-hidden /> Input
        </span>
        <pre className="eth-ai__pre">{pretty(item.input)}</pre>
        {item.result && (
          <>
            <span className="eth-ai__tool-label">{item.result.is_error ? "Error" : "Result"}</span>
            <pre className="eth-ai__pre">{pretty(item.result.content)}</pre>
          </>
        )}
      </div>
    </details>
  );
}

function Composer() {
  const transport = useContext(TransportContext)?.transport ?? null;
  const running = useAiChat((s) => s.running);
  const [text, setText] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  const focus = useAiFocus((s) => s.request);
  useEffect(() => {
    input.current?.focus();
  }, [focus]);
  const send = () => {
    if (!transport || running || !text.trim()) return;
    void sendMessage(transport, text);
    setText("");
  };
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      send();
    } else if (e.key === "Escape" && running) {
      e.preventDefault();
      e.stopPropagation();
      stopTurn();
    }
  };
  return (
    <div className="eth-ai__composer">
      <textarea
        ref={input}
        className="eth-input eth-ai__input"
        aria-label="Ask AI"
        placeholder={`Ask AI to edit your project (${MOD_KEY}⇧A)`}
        rows={2}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        data-testid="ai-input"
      />
      <div className="eth-ai__footer">
        <span className="eth-ai__hint">{running ? "Esc or Stop to interrupt" : "Enter to send, Shift+Enter for a new line"}</span>
        {running ? (
          <Button size="sm" tone="danger" onClick={stopTurn} data-testid="ai-stop">
            <Square aria-hidden className="eth-ai__btn-icon" /> Stop
          </Button>
        ) : (
          <Button size="sm" tone="accent" disabled={!transport || !text.trim()} onClick={send} data-testid="ai-send">
            <ArrowUp aria-hidden className="eth-ai__btn-icon" /> Send
          </Button>
        )}
      </div>
    </div>
  );
}
