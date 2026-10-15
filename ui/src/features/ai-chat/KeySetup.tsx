// Inline provider + API key setup (first run, or after the provider rejected the key).
import clsx from "clsx";
import { KeyRound, Sparkles } from "lucide-react";
import { useState } from "react";
import { Button, Select, TextInput } from "@/kit";
import { useT } from "@/i18n";
import { isTauri } from "@/transport";
import { useAiChat } from "./chatStore";
import { hostOf, keyHelpText, keyPlaceholder, providerName, providerOptions, PROVIDERS, type ProviderId } from "./providers";
import { looksLikeApiKey, useAiSettings } from "./settings";

/** Where keys live, and where they go. */
export function KeyNote({ provider }: { provider: ProviderId }) {
  const t = useT();
  const baseUrl = useAiSettings((s) => s.configs[provider].baseUrl);
  const host = hostOf(baseUrl);
  return (
    <p className="eth-ai__hint eth-ai__note">
      <KeyRound aria-hidden className="eth-ai__note-icon" />
      {isTauri() ? t("ai_chat.key.stored_desktop", { host }) : t("ai_chat.key.stored_web", { host })}
    </p>
  );
}

/** The key field of `provider` (Save, with a soft format check). */
export function KeyField({ provider, onSaved }: { provider: ProviderId; onSaved?(): void }) {
  const t = useT();
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
          aria-label={t("ai_chat.key.label")}
          placeholder={keyPlaceholder(p)}
          // The key's format ("sk-ant-…") isn't text to translate.
          data-i18n-exempt={p.keyHint ? true : undefined}
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
          {warn ? t("ai_chat.key.save_anyway") : t("ai_chat.key.save")}
        </Button>
      </div>
      {warn && (
        <p className="eth-ai__hint eth-ai__hint--warn">
          {provider === "anthropic"
            ? t("ai_chat.key.unlike_format", { provider: "Anthropic", format: p.keyHint })
            : p.keyHint.endsWith("…")
              ? t("ai_chat.key.unlike_format", { provider: providerName(provider), format: p.keyHint })
              : t("ai_chat.key.unlike", { provider: providerName(provider) })}
        </p>
      )}
    </>
  );
}

/** First run (pick a provider, enter its key), or the key was rejected. */
export function KeySetup({ rejected = false }: { rejected?: boolean }) {
  const t = useT();
  const provider = useAiSettings((s) => s.provider);
  const setProvider = useAiSettings((s) => s.setProvider);
  const p = PROVIDERS[provider];
  return (
    <div className={clsx("eth-ai__setup", rejected && "eth-ai__setup--rejected")} role="form" aria-label={t("ai_chat.setup.connect")}>
      {!rejected && (
        <div className="eth-ai__setup-icon" aria-hidden>
          <Sparkles />
        </div>
      )}
      <p className="eth-ai__setup-title">{rejected ? t("ai_chat.setup.rejected") : t("ai_chat.composer.placeholder")}</p>
      {!rejected && (
        <>
          <p className="eth-ai__hint">{t("ai_chat.setup.intro")}</p>
          <Select<ProviderId>
            size="sm"
            aria-label={t("ai_chat.provider.label")}
            options={providerOptions()}
            value={provider}
            onChange={setProvider}
            data-testid="ai-setup-provider"
          />
        </>
      )}
      <KeyField provider={provider} />
      {/* A provider's site and its own labels ("console.anthropic.com → API keys") stay as written. */}
      <p className="eth-ai__hint" data-i18n-exempt={"site" in p.keyHelp || undefined}>
        {keyHelpText(p)}
      </p>
      <KeyNote provider={provider} />
    </div>
  );
}
