// Inline API key setup (first run, or after the API rejected the key).
import clsx from "clsx";
import { KeyRound, Sparkles } from "lucide-react";
import { useState } from "react";
import { Button, TextInput } from "@/kit";
import { isTauri } from "@/transport";
import { useAiChat } from "./chatStore";
import { looksLikeApiKey, useAiSettings } from "./settings";

/** Inline API key setup (first run, or after the API rejected the key). */
export function KeySetup({ rejected = false }: { rejected?: boolean }) {
  const setApiKey = useAiSettings((s) => s.setApiKey);
  const [draft, setDraft] = useState("");
  const [warn, setWarn] = useState(false);
  const save = () => {
    const k = draft.trim();
    if (!k) return;
    if (!looksLikeApiKey(k) && !warn) {
      setWarn(true);
      return;
    }
    setApiKey(k);
    setDraft("");
    setWarn(false);
    useAiChat.setState({ keyRejected: false });
  };
  return (
    <form
      className={clsx("eth-ai__setup", rejected && "eth-ai__setup--rejected")}
      aria-label="Connect Claude"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      {!rejected && (
        <div className="eth-ai__setup-icon" aria-hidden>
          <Sparkles />
        </div>
      )}
      <p className="eth-ai__setup-title">{rejected ? "Your API key was rejected" : "Ask AI to edit your project"}</p>
      {!rejected && (
        <p className="eth-ai__hint">
          Claude can create tracks, write notes, set the mix and more. Every change is a normal undo step. Uses your own Anthropic API key
          (console.anthropic.com → API keys).
        </p>
      )}
      <div className="eth-ai__setup-row">
        <TextInput
          type="password"
          size="sm"
          className="eth-ai__setup-input"
          aria-label="API key"
          placeholder="sk-ant-…"
          autoComplete="off"
          spellCheck={false}
          value={draft}
          invalid={warn}
          onChange={(e) => {
            setDraft(e.target.value);
            setWarn(false);
          }}
        />
        <Button type="submit" size="sm" tone="accent" disabled={!draft.trim()}>
          {warn ? "Save anyway" : "Save key"}
        </Button>
      </div>
      {warn && <p className="eth-ai__hint eth-ai__hint--warn">That doesn't look like an Anthropic key (sk-ant-…).</p>}
      <p className="eth-ai__hint eth-ai__note">
        <KeyRound aria-hidden className="eth-ai__note-icon" />
        {isTauri()
          ? "Stored on this computer in Ethereal's app data. It is only ever sent to api.anthropic.com."
          : "Stored in this browser's local storage (anyone using this browser profile can read it). It is only ever sent to api.anthropic.com."}
      </p>
    </form>
  );
}
