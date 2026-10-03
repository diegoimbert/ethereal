import "./join.css";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { Button, Dialog, TextInput } from "@/kit";
import { linkProblem } from "./links";
import { openInvite, useJoinStore } from "./store";

export interface JoinWithLinkFieldProps {
  /** After the link was handed to the engine (e.g. close the popover holding the field). */
  onSubmitted?: () => void;
  /** Focus the field on mount. */
  autoFocus?: boolean;
}

/**
 * "Join with a link…": paste any invite form (`https://…/join/…#…` or `ethereal://join/…`).
 * The join screen takes over once the engine has the link. Used by the Share popover
 * (`share-ui`) and the dialog below.
 */
export function JoinWithLinkField({ onSubmitted, autoFocus }: JoinWithLinkFieldProps) {
  const [link, setLink] = useState("");
  const [error, setError] = useState<string | null>(null);
  const form = useRef<HTMLFormElement>(null);
  // After a containing dialog focused itself.
  useEffect(() => {
    if (!autoFocus) return;
    const t = setTimeout(() => form.current?.querySelector("input")?.focus(), 0);
    return () => clearTimeout(t);
  }, [autoFocus]);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const problem = linkProblem(link);
    setError(problem);
    if (problem) return;
    openInvite(link.trim());
    setLink("");
    onSubmitted?.();
  };
  return (
    <form ref={form} className="eth-join__paste" onSubmit={submit} data-testid="join-with-link">
      <div className="eth-join__paste-row">
        <TextInput
          aria-label="Invite link"
          placeholder="Paste an invite link"
          value={link}
          invalid={!!error}
          onChange={(e) => {
            setLink(e.target.value);
            setError(null);
          }}
        />
        <Button type="submit" tone="accent" disabled={!link.trim()}>
          Join
        </Button>
      </div>
      {error && (
        <p className="eth-join__error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

/** The "Join with a link…" dialog (`openJoinWithLink()`). */
export function JoinWithLinkDialog() {
  const open = useJoinStore((s) => s.pasteOpen);
  const setOpen = useJoinStore((s) => s.setPasteOpen);
  return (
    <Dialog open={open} onClose={() => setOpen(false)} title="Join with a link" footer={<Button onClick={() => setOpen(false)}>Cancel</Button>}>
      {open && <JoinWithLinkField autoFocus onSubmitted={() => setOpen(false)} />}
    </Dialog>
  );
}
