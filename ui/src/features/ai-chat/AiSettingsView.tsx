// AI settings: the API key, the per-turn tool-round cap, and (desktop) the MCP bridge toggle.
import { Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { Button, IconButton, NumberField, Toggle } from "@/kit";
import { isTauri } from "@/transport";
import { KeySetup } from "./KeySetup";
import { bridgeStatus, CLAUDE_MCP_ADD, setBridgeEnabled, type BridgeStatus } from "./mcpBridge";
import { DEFAULT_MAX_ITERATIONS, maskKey, MAX_ITERATIONS_LIMIT, useAiSettings } from "./settings";

export function AiSettingsView({ onDone }: { onDone(): void }) {
  const apiKey = useAiSettings((s) => s.apiKey);
  const setApiKey = useAiSettings((s) => s.setApiKey);
  const maxIterations = useAiSettings((s) => s.maxIterations);
  const setMaxIterations = useAiSettings((s) => s.setMaxIterations);
  const [changing, setChanging] = useState(false);
  return (
    <div className="eth-ai__settings" data-testid="ai-settings">
      <section className="eth-ai__section" aria-label="API key">
        <h3 className="eth-ai__heading">Anthropic API key</h3>
        {apiKey && !changing ? (
          <div className="eth-ai__row">
            <code className="eth-ai__key" data-testid="ai-key-masked">
              {maskKey(apiKey)}
            </code>
            <span className="eth-ai__spacer" />
            <Button size="sm" onClick={() => setChanging(true)}>
              Change
            </Button>
            <Button size="sm" tone="danger" onClick={() => setApiKey(null)}>
              Remove
            </Button>
          </div>
        ) : (
          <KeySetupInline onSaved={() => setChanging(false)} />
        )}
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
          <span className="eth-ai__hint">
            The AI pauses after this many rounds of tool calls in one reply (default {DEFAULT_MAX_ITERATIONS}).
          </span>
        </div>
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

function KeySetupInline({ onSaved }: { onSaved(): void }) {
  const apiKey = useAiSettings((s) => s.apiKey);
  const [initial] = useState(apiKey);
  useEffect(() => {
    if (apiKey !== initial) onSaved();
  }, [apiKey, initial, onSaved]);
  return <KeySetup />;
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
