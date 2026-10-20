// The AI chat (left rail tab "Ask AI"): talk to a model (Anthropic, or any OpenAI-compatible
// provider), which edits the open project through the agent API (the same tools as the MCP
// server). BYOK: the user's own API key, or a local server.
import "./aiChat.css";
import clsx from "clsx";
import { ArrowUp, Check, CircleAlert, Eraser, LoaderCircle, Settings, ShieldCheck, Square, Wrench, X } from "lucide-react";
import { useContext, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { Trans, useT, type MessageKey } from "@/i18n";
import { Badge, Button, IconButton, Select } from "@/kit";
import { useShortcutLabel } from "@/features/keymap";
import { useProjectStore } from "@/state";
import { TransportContext } from "@/transport";
import { AiSettingsView } from "./AiSettingsView";
import { KeySetup } from "./KeySetup";
import { newChat, sendMessage, stopTurn, useAiChat, type ChatItem } from "./chatStore";
import { useAiFocus } from "./open";
import { PROVIDERS, providerName } from "./providers";
import { hasConsent, missingSetup, modelLabel, useAiSettings } from "./settings";
import { pretty, summarizeInput, summarizeResult } from "./toolSummary";

/** Requests the empty chat suggests (sent as typed, in the current language). */
const EXAMPLES: readonly MessageKey[] = ["ai_chat.example.arpeggio", "ai_chat.example.tempo", "ai_chat.example.chords"];

export function AiChatPanel() {
  const t = useT();
  const missing = useAiSettings((s) => missingSetup(s));
  const providerLabel = useAiSettings((s) => providerName(s.provider));
  const keyRejected = useAiChat((s) => s.keyRejected);
  const consented = useAiSettings((s) => hasConsent(s));
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
          <p className="eth-ai__setup-title">{t("ai_chat.setup.choose_model")}</p>
          <p className="eth-ai__hint">{t("ai_chat.setup.model_hint", { provider: providerLabel })}</p>
          <Button size="sm" tone="accent" onClick={() => setSettings(true)}>
            {t("ai_chat.setup.open_settings")}
          </Button>
        </div>
      ) : !consented ? (
        <Consent />
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
  const t = useT();
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
        aria-label={t("ai_chat.model")}
        className="eth-ai__model"
        options={models.map((m) => ({ value: m, label: modelLabel(m), group: providerName(provider), userText: true }))}
        value={model}
        placeholder={t("ai_chat.setup.choose_model")}
        onChange={setModel}
        data-testid="ai-model"
      />
      <AlphaBadge />
      <span className="eth-ai__spacer" />
      <IconButton size="sm" tone="ghost" label={t("ai_chat.clear")} icon={<Eraser />} disabled={empty} onClick={newChat} />
      <IconButton size="sm" tone="ghost" label={t("ai_chat.settings.title")} icon={<Settings />} active={settings} onClick={onSettings} />
    </div>
  );
}

/** The assistant is in alpha; always shown in the toolbar, the notice as its tooltip. */
function AlphaBadge() {
  const t = useT();
  return (
    <span className="eth-ai__alpha" title={t("ai_chat.alpha_notice")} data-testid="ai-alpha-badge">
      <Badge>{t("ai_chat.alpha")}</Badge>
    </span>
  );
}

/**
 * What "alpha" means. In the first-use card and the empty conversation; above a
 * conversation, `dismissible` (remembered with the AI settings: per user, not per project).
 */
function AlphaNotice({ dismissible }: { dismissible?: boolean }) {
  const t = useT();
  return (
    <div className={clsx("eth-ai__alpha-note", dismissible && "eth-ai__alpha-note--bar")} role="note" data-testid="ai-alpha-notice">
      <p className="eth-ai__alpha-text">{t("ai_chat.alpha_notice")}</p>
      {dismissible && (
        <IconButton
          size="sm"
          tone="ghost"
          label={t("common.dismiss")}
          icon={<X />}
          onClick={() => useAiSettings.getState().dismissAlpha()}
          data-testid="ai-alpha-dismiss"
        />
      )}
    </div>
  );
}

/**
 * Before the first message to a provider, say what leaves the machine. Asked once per
 * provider (`useAiSettings().consented`, kept with the AI settings).
 */
function Consent() {
  const t = useT();
  const provider = useAiSettings((s) => s.provider);
  const p = PROVIDERS[provider];
  const name = providerName(provider);
  return (
    <div className="eth-ai__setup" data-testid="ai-consent">
      <ShieldCheck className="eth-ai__setup-icon" aria-hidden />
      <p className="eth-ai__setup-title">{t("ai_chat.consent.title")}</p>
      <AlphaNotice />
      <p className="eth-ai__hint">
        <Trans k="ai_chat.consent.sends" values={{ provider: <strong className="eth-user-text">{name}</strong> }} />{" "}
        {p.keyRequired ? t("ai_chat.consent.cloud") : t("ai_chat.consent.local")}
      </p>
      <p className="eth-ai__hint">{t("ai_chat.consent.kept")}</p>
      <Button size="sm" tone="accent" onClick={() => useAiSettings.getState().consent(provider)} data-testid="ai-consent-accept">
        {t("ai_chat.consent.accept", { provider: name })}
      </Button>
    </div>
  );
}

function Messages() {
  const t = useT();
  const items = useAiChat((s) => s.items);
  const running = useAiChat((s) => s.running);
  const alphaDismissed = useAiSettings((s) => s.alphaDismissed);
  const ref = useRef<HTMLOListElement>(null);
  const stick = useRef(true);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [items]);
  if (items.length === 0) return <EmptyState />;
  const last = items.at(-1);
  return (
    <>
      {!alphaDismissed && <AlphaNotice dismissible />}
      <ol
        ref={ref}
        className="eth-ai__list"
        aria-label={t("ai_chat.conversation")}
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
            <LoaderCircle className="eth-ai__spin" aria-hidden /> {t("ai_chat.working")}
          </li>
        )}
      </ol>
    </>
  );
}

function EmptyState() {
  const t = useT();
  const transport = useContext(TransportContext)?.transport ?? null;
  return (
    <div className="eth-ai__empty">
      <AlphaNotice />
      <p className="eth-ai__hint">{t("ai_chat.empty_hint")}</p>
      {EXAMPLES.map((key) => (
        <button key={key} type="button" className="eth-ai__example" disabled={!transport} onClick={() => transport && void sendMessage(transport, t(key))}>
          {t(key)}
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
          <p className="eth-ai__text eth-user-text">{item.text}</p>
        </li>
      );
    case "assistant":
      return (
        <li className="eth-ai__msg eth-ai__msg--assistant" data-testid="ai-message" data-role="assistant">
          <p className="eth-ai__text eth-user-text">{item.text}</p>
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
  const t = useT();
  const state = item.result === null ? "running" : item.result.is_error ? "error" : "ok";
  const icon = state === "running" ? <LoaderCircle className="eth-ai__spin" /> : state === "error" ? <CircleAlert /> : <Check />;
  return (
    <details className={clsx("eth-ai__tool", `eth-ai__tool--${state}`)} data-testid="ai-tool" data-tool={item.name} data-state={state}>
      <summary className="eth-ai__tool-summary">
        <span className="eth-ai__tool-icon" aria-hidden>
          {icon}
        </span>
        <span className="eth-ai__tool-name">{item.name}</span>
        <span className="eth-ai__tool-input eth-user-text">{summarizeInput(item.input)}</span>
        {item.result && <span className="eth-ai__tool-result">{item.result.is_error ? summarizeResult(item.result.content) : ""}</span>}
      </summary>
      <div className="eth-ai__tool-details">
        <span className="eth-ai__tool-label">
          <Wrench aria-hidden /> {t("ai_chat.tool.input")}
        </span>
        <pre className="eth-ai__pre eth-user-text">{pretty(item.input)}</pre>
        {item.result && (
          <>
            <span className="eth-ai__tool-label">{item.result.is_error ? t("ai_chat.tool.error") : t("ai_chat.tool.result")}</span>
            <pre className="eth-ai__pre eth-user-text">{pretty(item.result.content)}</pre>
          </>
        )}
      </div>
    </details>
  );
}

function Composer() {
  const t = useT();
  const transport = useContext(TransportContext)?.transport ?? null;
  // Nothing to edit before a project is open (the app launches with none).
  const hasProject = useProjectStore((s) => s.project !== null);
  const running = useAiChat((s) => s.running);
  const [text, setText] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  const askKey = useShortcutLabel("ai:ask");
  const focus = useAiFocus((s) => s.request);
  useEffect(() => {
    input.current?.focus();
  }, [focus]);
  const send = () => {
    if (!transport || !hasProject || running || !text.trim()) return;
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
        aria-label={t("ai_chat.composer.label")}
        placeholder={
          hasProject
            ? askKey
              ? t("ai_chat.composer.placeholder_shortcut", { shortcut: askKey })
              : t("ai_chat.composer.placeholder")
            : t("ai_chat.composer.no_project")
        }
        disabled={!hasProject && !running}
        rows={2}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        data-testid="ai-input"
      />
      <div className="eth-ai__footer">
        <span className="eth-ai__hint">{running ? t("ai_chat.composer.running_hint") : t("ai_chat.composer.hint")}</span>
        {running ? (
          <Button size="sm" tone="danger" onClick={stopTurn} data-testid="ai-stop">
            <Square aria-hidden className="eth-ai__btn-icon" /> {t("ai_chat.composer.stop")}
          </Button>
        ) : (
          <Button size="sm" tone="accent" disabled={!transport || !hasProject || !text.trim()} onClick={send} data-testid="ai-send">
            <ArrowUp aria-hidden className="eth-ai__btn-icon" /> {t("ai_chat.composer.send")}
          </Button>
        )}
      </div>
    </div>
  );
}
