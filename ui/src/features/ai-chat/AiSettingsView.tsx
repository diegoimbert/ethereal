// AI settings: the provider (base URL, model, API key, test connection), the per-turn
// tool-round cap, and (desktop) the MCP bridge toggle.
import { Copy, RotateCcw } from "lucide-react";
import { useEffect, useId, useState, type ReactNode } from "react";
import { Button, IconButton, NumberField, Select, TextInput, Toggle } from "@/kit";
import { isTauri } from "@/transport";
import { KeyField, KeyNote } from "./KeySetup";
import { bridgeStatus, CLAUDE_MCP_ADD, setBridgeEnabled, type BridgeStatus } from "./mcpBridge";
import { PROVIDER_OPTIONS, PROVIDERS, testConnection, type ProviderId } from "./providers";
import { activeConnection, DEFAULT_MAX_ITERATIONS, maskKey, MAX_ITERATIONS_LIMIT, useAiSettings } from "./settings";

export function AiSettingsView({ onDone }: { onDone(): void }) {
  const provider = useAiSettings((s) => s.provider);
  const setProvider = useAiSettings((s) => s.setProvider);
  const maxIterations = useAiSettings((s) => s.maxIterations);
  const setMaxIterations = useAiSettings((s) => s.setMaxIterations);
  return (
    <div className="eth-ai__settings" data-testid="ai-settings">
      <section className="eth-ai__section" aria-label="Provider">
        <h3 className="eth-ai__heading">Provider</h3>
        <Select<ProviderId> size="sm" aria-label="Provider" options={PROVIDER_OPTIONS} value={provider} onChange={setProvider} data-testid="ai-provider" />
        <ProviderFields key={provider} provider={provider} />
      </section>

      <section className="eth-ai__section" aria-label="Tool rounds">
        <h3 className="eth-ai__heading">Tool rounds per message</h3>
        <div className="eth-ai__row">
          <NumberField
            size="sm"
            aria-label="Tool rounds per message"
            className="eth-ai__number"
            value={maxIterations}
            min={1}
            max={MAX_ITERATIONS_LIMIT}
            step={1}
            precision={0}
            onChange={setMaxIterations}
          />
        </div>
        <p className="eth-ai__hint">The AI pauses after this many rounds of tool calls in one reply (default {DEFAULT_MAX_ITERATIONS}).</p>
      </section>

      {isTauri() && <McpSection />}

      <div className="eth-ai__row eth-ai__done">
        <span className="eth-ai__spacer" />
        <Button size="sm" tone="accent" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}

/** Base URL, model, key and "Test connection" of the active provider. */
function ProviderFields({ provider }: { provider: ProviderId }) {
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
      <Field label="Base URL">
        <CommitInput key={config.baseUrl} label="Base URL" value={config.baseUrl} onCommit={setBaseUrl} placeholder={p.baseUrl} />
        {config.baseUrl !== p.baseUrl && (
          <IconButton size="sm" tone="ghost" label="Reset base URL" icon={<RotateCcw />} onClick={() => setBaseUrl(p.baseUrl)} />
        )}
      </Field>
      <Field label="Model">
        <CommitInput key={config.model} label="Model name" value={config.model} onCommit={setModel} placeholder={p.models[0] ?? "model name"} list={listId} />
        <datalist id={listId}>
          {p.models.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
      </Field>
      <Field label={p.keyRequired ? "API key" : "API key (optional)"}>
        {apiKey && !changingKey ? (
          <>
            <code className="eth-ai__key" data-testid="ai-key-masked">
              {maskKey(apiKey)}
            </code>
            <span className="eth-ai__spacer" />
            <Button size="sm" onClick={() => setChangingKey(true)}>
              Change
            </Button>
            <Button size="sm" tone="danger" onClick={() => setApiKey(null, provider)}>
              Remove
            </Button>
          </>
        ) : (
          <div className="eth-ai__field-stack">
            <KeyField provider={provider} onSaved={() => setChangingKey(false)} />
          </div>
        )}
      </Field>
      <p className="eth-ai__hint">{p.keyHelp}</p>
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
function CommitInput({ label, value, onCommit, placeholder, list }: { label: string; value: string; onCommit(v: string): void; placeholder?: string; list?: string }) {
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
  const [last, setT] = useState<TestState>({ state: "idle" });
  const config = useAiSettings((s) => s.configs[provider]);
  const apiKey = useAiSettings((s) => s.keys[provider]);
  // A result is for the settings it tested: changing them hides it.
  const sig = JSON.stringify([config.baseUrl, config.model, apiKey]);
  const t: TestState = last.state !== "idle" && last.sig !== sig ? { state: "idle" } : last;
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
        <Button size="sm" onClick={run} disabled={t.state === "busy" || blocked || !config.model} data-testid="ai-test-connection">
          {t.state === "busy" ? "Testing…" : "Test connection"}
        </Button>
      </div>
      {t.state === "ok" && (
        <p className="eth-ai__hint eth-ai__hint--ok" data-testid="ai-test-result" role="status">
          {t.text}
        </p>
      )}
      {t.state === "error" && (
        <p className="eth-ai__hint eth-ai__hint--warn" data-testid="ai-test-result" role="alert">
          {t.text}
        </p>
      )}
    </div>
  );
}

/** Desktop only: let external agents (Claude Code, Claude Desktop) drive the app over MCP. */
export function McpSection() {
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
    const t = setInterval(() => void poll(), 2000);
    return () => {
      alive = false;
      clearInterval(t);
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
  const copy = () => {
    void navigator.clipboard?.writeText(CLAUDE_MCP_ADD).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  const enabled = status?.enabled ?? false;
  return (
    <section className="eth-ai__section" aria-label="External AI agents" data-testid="ai-mcp">
      <h3 className="eth-ai__heading">External AI agents</h3>
      <Toggle
        checked={enabled}
        disabled={busy || status === null}
        onChange={toggle}
        label="Allow external AI agents (MCP)"
      />
      <p className="eth-ai__hint">
        Lets AI apps on this computer (Claude Code, Claude Desktop) edit the open project through Ethereal's MCP server. Local connections
        only, with a per-session token.
      </p>
      {enabled && status && (
        <>
          <p className="eth-ai__hint" data-testid="ai-mcp-status">
            Listening on 127.0.0.1:{status.port ?? "…"} · {status.connected_clients} connected
          </p>
          <div className="eth-ai__row">
            <code className="eth-ai__cmd">{CLAUDE_MCP_ADD}</code>
            <IconButton size="sm" tone="ghost" label={copied ? "Copied" : "Copy command"} icon={<Copy />} onClick={copy} />
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
