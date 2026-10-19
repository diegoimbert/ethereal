// AI settings: the provider (base URL, model, API key, test connection), the per-turn
// tool-round cap, and (desktop) the MCP bridge toggle.
import { Copy, RotateCcw } from "lucide-react";
import { useEffect, useId, useState, type ReactNode } from "react";
import { Button, IconButton, NumberField, Select, TextInput, Toggle } from "@/kit";
import { isTauri } from "@/transport";
import { useT } from "@/i18n";
import { KeyField, KeyNote } from "./KeySetup";
import { bridgeStatus, claudeMcpAdd, setBridgeEnabled, type BridgeStatus } from "./mcpBridge";
import { keyHelpText, providerOptions, PROVIDERS, testConnection, type ProviderId } from "./providers";
import { activeConnection, DEFAULT_MAX_ITERATIONS, maskKey, MAX_ITERATIONS_LIMIT, useAiSettings } from "./settings";

export function AiSettingsView({ onDone }: { onDone(): void }) {
  const t = useT();
  const provider = useAiSettings((s) => s.provider);
  const setProvider = useAiSettings((s) => s.setProvider);
  const maxIterations = useAiSettings((s) => s.maxIterations);
  const setMaxIterations = useAiSettings((s) => s.setMaxIterations);
  return (
    <div className="eth-ai__settings" data-testid="ai-settings">
      <section className="eth-ai__section" aria-label={t("ai_chat.provider.label")}>
        <h3 className="eth-ai__heading">{t("ai_chat.provider.label")}</h3>
        <Select<ProviderId>
          size="sm"
          aria-label={t("ai_chat.provider.label")}
          options={providerOptions()}
          value={provider}
          onChange={setProvider}
          data-testid="ai-provider"
        />
        <ProviderFields key={provider} provider={provider} />
      </section>

      <section className="eth-ai__section" aria-label={t("ai_chat.settings.tool_rounds")}>
        <h3 className="eth-ai__heading">{t("ai_chat.settings.tool_rounds_per_message")}</h3>
        <div className="eth-ai__row">
          <NumberField
            size="sm"
            aria-label={t("ai_chat.settings.tool_rounds_per_message")}
            className="eth-ai__number"
            value={maxIterations}
            min={1}
            max={MAX_ITERATIONS_LIMIT}
            step={1}
            precision={0}
            onChange={setMaxIterations}
          />
        </div>
        <p className="eth-ai__hint">{t("ai_chat.settings.tool_rounds_hint", { count: DEFAULT_MAX_ITERATIONS })}</p>
      </section>

      {isTauri() && <McpSection />}

      <div className="eth-ai__row eth-ai__done">
        <span className="eth-ai__spacer" />
        <Button size="sm" tone="accent" onClick={onDone}>
          {t("common.done")}
        </Button>
      </div>
    </div>
  );
}

/** Base URL, model, key and "Test connection" of the active provider. */
function ProviderFields({ provider }: { provider: ProviderId }) {
  const t = useT();
  const p = PROVIDERS[provider];
  const config = useAiSettings((s) => s.configs[provider]);
  const apiKey = useAiSettings((s) => s.keys[provider]);
  const setBaseUrl = useAiSettings((s) => s.setBaseUrl);
  const setModel = useAiSettings((s) => s.setModel);
  const setApiKey = useAiSettings((s) => s.setApiKey);
  const [changingKey, setChangingKey] = useState(false);
  const listId = useId();
  return (
    <>
      <Field label={t("ai_chat.settings.base_url")}>
        <CommitInput
          key={config.baseUrl}
          label={t("ai_chat.settings.base_url")}
          value={config.baseUrl}
          onCommit={setBaseUrl}
          placeholder={p.baseUrl}
          dataPlaceholder
        />
        {config.baseUrl !== p.baseUrl && (
          <IconButton size="sm" tone="ghost" label={t("ai_chat.settings.reset_base_url")} icon={<RotateCcw />} onClick={() => setBaseUrl(p.baseUrl)} />
        )}
      </Field>
      <Field label={t("ai_chat.model")}>
        <CommitInput
          key={config.model}
          label={t("ai_chat.settings.model_name")}
          value={config.model}
          onCommit={setModel}
          placeholder={p.models[0] ?? t("ai_chat.settings.model_name")}
          dataPlaceholder={p.models[0] !== undefined}
          list={listId}
        />
        <datalist id={listId}>
          {p.models.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
      </Field>
      <Field label={p.keyRequired ? t("ai_chat.key.label") : t("ai_chat.key.label_optional")}>
        {apiKey && !changingKey ? (
          <>
            <code className="eth-ai__key eth-user-text" data-testid="ai-key-masked">
              {maskKey(apiKey)}
            </code>
            <span className="eth-ai__spacer" />
            <Button size="sm" onClick={() => setChangingKey(true)}>
              {t("ai_chat.key.change")}
            </Button>
            <Button size="sm" tone="danger" onClick={() => setApiKey(null, provider)}>
              {t("ai_chat.key.remove")}
            </Button>
          </>
        ) : (
          <div className="eth-ai__field-stack">
            <KeyField provider={provider} onSaved={() => setChangingKey(false)} />
          </div>
        )}
      </Field>
      {/* A provider's site and its own labels ("console.anthropic.com → API keys") stay as written. */}
      <p className="eth-ai__hint" data-i18n-exempt={"site" in p.keyHelp || undefined}>
        {keyHelpText(p)}
      </p>
      <KeyNote provider={provider} />
      <TestConnection provider={provider} />
    </>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="eth-ai__field">
      <span className="eth-ai__label">{label}</span>
      <div className="eth-ai__row">{children}</div>
    </div>
  );
}

/** A text field (keyed on its value) that commits on Enter or blur (Escape reverts). */
function CommitInput({
  label,
  value,
  onCommit,
  placeholder,
  dataPlaceholder,
  list,
}: {
  label: string;
  value: string;
  onCommit(v: string): void;
  placeholder?: string;
  /** The placeholder is data (a URL, a model id), not text to translate. */
  dataPlaceholder?: boolean;
  list?: string;
}) {
  // Keyed on `value` by the caller: a new value resets the draft.
  const [draft, setDraft] = useState(value);
  const commit = () => {
    if (draft.trim() !== value) onCommit(draft);
  };
  return (
    <TextInput
      size="sm"
      className="eth-ai__setup-input"
      aria-label={label}
      value={draft}
      placeholder={placeholder}
      data-i18n-exempt={dataPlaceholder || undefined}
      list={list}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commit();
        } else if (e.key === "Escape") {
          e.stopPropagation();
          setDraft(value);
        }
      }}
    />
  );
}

type TestState = { state: "idle" } | { state: "busy"; sig: string } | { state: "ok" | "error"; text: string; sig: string };

function TestConnection({ provider }: { provider: ProviderId }) {
  const t = useT();
  const [last, setT] = useState<TestState>({ state: "idle" });
  const config = useAiSettings((s) => s.configs[provider]);
  const apiKey = useAiSettings((s) => s.keys[provider]);
  // A result is for the settings it tested: changing them hides it.
  const sig = JSON.stringify([config.baseUrl, config.model, apiKey]);
  const shown: TestState = last.state !== "idle" && last.sig !== sig ? { state: "idle" } : last;
  const run = () => {
    setT({ state: "busy", sig });
    const conn = activeConnection({ ...useAiSettings.getState(), provider });
    testConnection(provider, conn)
      .then((text) => setT({ state: "ok", text, sig }))
      .catch((e: unknown) => setT({ state: "error", text: e instanceof Error ? e.message : String(e), sig }));
  };
  const blocked = PROVIDERS[provider].keyRequired && !apiKey;
  return (
    <div className="eth-ai__field">
      <div className="eth-ai__row">
        <Button size="sm" onClick={run} disabled={shown.state === "busy" || blocked || !config.model} data-testid="ai-test-connection">
          {shown.state === "busy" ? t("ai_chat.settings.testing") : t("ai_chat.settings.test")}
        </Button>
      </div>
      {shown.state === "ok" && (
        <p className="eth-ai__hint eth-ai__hint--ok" data-testid="ai-test-result" role="status">
          {shown.text}
        </p>
      )}
      {shown.state === "error" && (
        <p className="eth-ai__hint eth-ai__hint--warn" data-testid="ai-test-result" role="alert">
          {shown.text}
        </p>
      )}
    </div>
  );
}

/** Desktop only: let external agents (Claude Code, Claude Desktop) drive the app over MCP. */
export function McpSection() {
  const t = useT();
  const [status, setStatus] = useState<BridgeStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    let alive = true;
    const poll = () =>
      bridgeStatus()
        .then((s) => alive && setStatus(s))
        .catch((e: unknown) => alive && setError(String(e)));
    void poll();
    const timer = setInterval(() => void poll(), 2000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);
  const toggle = (enabled: boolean) => {
    setBusy(true);
    setError(null);
    setBridgeEnabled(enabled)
      .then(setStatus)
      .catch((e: unknown) => setError(String(e)))
      .finally(() => setBusy(false));
  };
  const command = claudeMcpAdd(status?.mcp_path);
  const copy = () => {
    void navigator.clipboard?.writeText(command).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  const enabled = status?.enabled ?? false;
  return (
    <section className="eth-ai__section" aria-label={t("ai_chat.mcp.title")} data-testid="ai-mcp">
      <h3 className="eth-ai__heading">{t("ai_chat.mcp.title")}</h3>
      <Toggle checked={enabled} disabled={busy || status === null} onChange={toggle} label={t("ai_chat.mcp.allow")} />
      <p className="eth-ai__hint">{t("ai_chat.mcp.hint", { apps: "claude code, claude desktop" })}</p>
      {enabled && status && (
        <>
          <p className="eth-ai__hint" data-testid="ai-mcp-status">
            {t("ai_chat.mcp.listening", { address: `127.0.0.1:${status.port ?? "…"}`, count: status.connected_clients })}
          </p>
          <div className="eth-ai__row">
            <code className="eth-ai__cmd eth-user-text">{command}</code>
            <IconButton size="sm" tone="ghost" label={copied ? t("ai_chat.mcp.copied") : t("ai_chat.mcp.copy")} icon={<Copy />} onClick={copy} />
          </div>
        </>
      )}
      {error && (
        <p className="eth-ai__hint eth-ai__hint--warn" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
