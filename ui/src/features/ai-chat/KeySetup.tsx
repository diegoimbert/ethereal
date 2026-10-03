// Inline provider + API key setup (first run, or after the provider rejected the key).
import clsx from "clsx";
import { KeyRound, Sparkles } from "lucide-react";
import { useState } from "react";
import { Button, Select, TextInput } from "@/kit";
import { isTauri } from "@/transport";
import { useAiChat } from "./chatStore";
import { hostOf, PROVIDER_OPTIONS, PROVIDERS, type ProviderId } from "./providers";
import { looksLikeApiKey, useAiSettings } from "./settings";

/** Where keys live, and where they go. */
export function KeyNote({ provider }: { provider: ProviderId }) {
  const baseUrl = useAiSettings((s) => s.configs[provider].baseUrl);
  const host = hostOf(baseUrl);
  return (
    <p className="eth-ai__hint eth-ai__note">
      <KeyRound aria-hidden className="eth-ai__note-icon" />
      {isTauri()
        ? `Stored on this computer in Ethereal's app data. It is only ever sent to ${host}.`
        : `Stored in this browser's local storage (anyone using this browser profile can read it). It is only ever sent to ${host}.`}
    </p>
  );
}

/** The key field of `provider` (Save, with a soft format check). */
export function KeyField({ provider, onSaved }: { provider: ProviderId; onSaved?(): void }) {
  const setApiKey = useAiSettings((s) => s.setApiKey);
  const p = PROVIDERS[provider];
  const [draft, setDraft] = useState("");
  const [warn, setWarn] = useState(false);
  const save = () => {
    const k = draft.trim();
    if (!k) return;
    if (!looksLikeApiKey(k, provider) && !warn) {
      setWarn(true);
      return;
    }
    setApiKey(k, provider);
    setDraft("");
    setWarn(false);
    useAiChat.setState({ keyRejected: false });
    onSaved?.();
  };
  return (
    <>
      <div className="eth-ai__setup-row">
        <TextInput
          type="password"
          size="sm"
          className="eth-ai__setup-input"
          aria-label="API key"
          placeholder={p.keyHint}
          autoComplete="off"
          spellCheck={false}
          value={draft}
          invalid={warn}
          onChange={(e) => {
            setDraft(e.target.value);
            setWarn(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              save();
            }
          }}
        />
        <Button size="sm" tone="accent" disabled={!draft.trim()} onClick={save}>
          {warn ? "Save anyway" : "Save key"}
        </Button>
      </div>
      {warn && (
        <p className="eth-ai__hint eth-ai__hint--warn">
          That doesn't look like {provider === "anthropic" ? "an Anthropic key (sk-ant-…)" : `a ${p.label} key${p.keyHint.endsWith("…") ? ` (${p.keyHint})` : ""}`}.
        </p>
      )}
    </>
  );
}

/** First run (pick a provider, enter its key), or the key was rejected. */
export function KeySetup({ rejected = false }: { rejected?: boolean }) {
  const provider = useAiSettings((s) => s.provider);
  const setProvider = useAiSettings((s) => s.setProvider);
  const p = PROVIDERS[provider];
  return (
    <div className={clsx("eth-ai__setup", rejected && "eth-ai__setup--rejected")} role="form" aria-label="Connect an AI provider">
      {!rejected && (
        <div className="eth-ai__setup-icon" aria-hidden>
          <Sparkles />
        </div>
      )}
      <p className="eth-ai__setup-title">{rejected ? "Your API key was rejected" : "Ask AI to edit your project"}</p>
      {!rejected && (
        <>
          <p className="eth-ai__hint">
            The AI can create tracks, write notes, set the mix and more. Every change is a normal undo step. Uses your own API key from the provider
            you pick, or a model running on this computer.
          </p>
          <Select<ProviderId> size="sm" aria-label="Provider" options={PROVIDER_OPTIONS} value={provider} onChange={setProvider} data-testid="ai-setup-provider" />
        </>
      )}
      <KeyField provider={provider} />
      <p className="eth-ai__hint">{p.keyHelp}</p>
      <KeyNote provider={provider} />
    </div>
  );
}
