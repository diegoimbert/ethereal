import "./join.css";
import { Loader2 } from "lucide-react";
import { useEffect, useState, type CSSProperties, type FormEvent, type ReactNode } from "react";
import { initials, peerColor } from "@/features/collab/store";
import type { InvitePreview, JoinStage, ParticipantSummary, ProjectId } from "@/generated";
import { Badge, Button, Dialog, TextInput } from "@/kit";
import { MAX_NAME_CHARS, validName } from "./identity";
import { failureText } from "./texts";

export interface JoinScreenProps {
  open: boolean;
  /** `ShareState::Joining.stage`, or a local error (`{ type: "Failed" }` built by the UI). */
  stage: JoinStage;
  /** The invite last shown as `Ready` (names the project while syncing). */
  invite: InvitePreview | null;
  /** Ask for a name ("Join as"): no sharing identity yet. */
  askName: boolean;
  /** Pressed Join: waiting for the engine. */
  busy?: boolean;
  /** Join (`AcceptInvite`), with the typed name when `askName`. */
  onJoin(name: string | null): void;
  /** Cancel, "Not now" or Close (`Leave`). */
  onDismiss(): void;
  /** "Open my offline copy" while the host is offline. */
  onOpenOfflineCopy?(project: ProjectId): void;
}

function Avatar({ who, size = "md" }: { who: ParticipantSummary; size?: "md" | "lg" }) {
  return (
    <span
      className={`eth-join__avatar eth-join__avatar--${size}`}
      style={{ "--eth-join-peer": peerColor(who.color) } as CSSProperties}
      title={who.name}
      aria-hidden
    >
      {initials(who.name)}
    </span>
  );
}

function Spinner({ label }: { label: string }) {
  return (
    <p className="eth-join__status" role="status">
      <Loader2 className="eth-join__spin" aria-hidden />
      {label}
    </p>
  );
}

function Progress({ received, total }: { received: number; total: number | null }) {
  const pct = total && total > 0 ? Math.min(100, Math.round((received / total) * 100)) : null;
  return (
    <div
      className={pct === null ? "eth-join__progress eth-join__progress--indeterminate" : "eth-join__progress"}
      role="progressbar"
      aria-label="Download progress"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct ?? undefined}
    >
      <span className="eth-join__progress-bar" style={pct === null ? undefined : ({ "--eth-join-progress": `${pct}%` } as CSSProperties)} />
    </div>
  );
}

/**
 * The join screen (docs/SHARING.md §8.3): a full-app dialog driven by
 * `ShareState::Joining.stage`, from "Connecting to the invite…" to
 * "<host> invites you to <project>: Join?", the download, or why it failed.
 */
export function JoinScreen({ open, stage, invite, askName, busy, onJoin, onDismiss, onOpenOfflineCopy }: JoinScreenProps) {
  const [name, setName] = useState("");
  const ready = stage.type === "Ready";

  // Join is the default action (the dialog focuses itself first, so focus after it).
  useEffect(() => {
    if (!open || !ready) return;
    const t = setTimeout(() => document.querySelector<HTMLElement>(".eth-join [data-autofocus]")?.focus(), 0);
    return () => clearTimeout(t);
  }, [open, ready, askName]);

  const canJoin = !busy && (!askName || validName(name));
  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    if (canJoin) onJoin(askName ? name.trim() : null);
  };

  let title = "Join a shared project";
  let body: ReactNode;
  let footer: ReactNode;
  switch (stage.type) {
    case "Contacting":
    case "Connecting":
      body = <Spinner label="Connecting to the invite…" />;
      footer = <Button onClick={onDismiss}>Cancel</Button>;
      break;
    case "Ready": {
      const inv = stage.invite;
      title = `Join “${inv.project_name}”`;
      const others = inv.online.filter((p) => p.name !== inv.host.name || p.color !== inv.host.color);
      body = (
        <form className="eth-join__ready" onSubmit={submit} data-testid="join-ready">
          <Avatar who={inv.host} size="lg" />
          <p className="eth-join__headline">
            <strong>{inv.host.name}</strong> invites you to <strong>{inv.project_name}</strong>
          </p>
          <Badge tone={inv.role === "Edit" ? "accent" : "default"}>{inv.role === "Edit" ? "You can edit" : "You can listen"}</Badge>
          {others.length > 0 && (
            <div className="eth-join__online" aria-label={`Also here: ${others.map((p) => p.name).join(", ")}`}>
              {others.slice(0, 5).map((p, i) => (
                <Avatar key={`${p.name}-${i}`} who={p} />
              ))}
              {others.length > 5 && <span className="eth-join__more">+{others.length - 5}</span>}
            </div>
          )}
          {askName && (
            <label className="eth-join__field">
              <span>Join as</span>
              <TextInput
                data-autofocus
                aria-label="Your name"
                placeholder="Your name"
                maxLength={MAX_NAME_CHARS}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
          )}
          {inv.local_copy && (
            <p className="eth-join__note">
              You have an older copy of {inv.project_name}. It will be updated, and any offline changes are kept separately.
            </p>
          )}
        </form>
      );
      footer = (
        <>
          <Button onClick={onDismiss}>Not now</Button>
          <Button data-autofocus={askName ? undefined : true} data-testid="join-accept" tone="accent" disabled={!canJoin} onClick={() => submit()}>
            {busy ? "Joining…" : "Join"}
          </Button>
        </>
      );
      break;
    }
    case "Syncing": {
      const project = invite?.project_name ?? "the project";
      title = invite ? `Join “${invite.project_name}”` : title;
      body = (
        <div className="eth-join__sync">
          <p className="eth-join__status">Downloading {project}…</p>
          <Progress received={stage.received_bytes} total={stage.total_bytes} />
        </div>
      );
      footer = <Button onClick={onDismiss}>Cancel</Button>;
      break;
    }
    case "HostOffline": {
      const copy = stage.local_copy;
      const project = invite?.project_name ?? "The project";
      body = (
        <div className="eth-join__sync">
          <p className="eth-join__text">The host's Ethereal is closed. {project} opens as soon as they're back.</p>
          <Spinner label="Waiting…" />
        </div>
      );
      footer = (
        <>
          {copy && onOpenOfflineCopy && <Button onClick={() => onOpenOfflineCopy(copy)}>Open my offline copy</Button>}
          <Button onClick={onDismiss}>Cancel</Button>
        </>
      );
      break;
    }
    case "Failed":
      title = "Can't join";
      body = (
        <p className="eth-join__error" role="alert" data-testid="join-error">
          {failureText(stage.reason, stage.message)}
        </p>
      );
      footer = (
        <Button tone="accent" onClick={onDismiss}>
          Close
        </Button>
      );
      break;
  }

  return (
    <Dialog open={open} onClose={stage.type === "Syncing" ? () => {} : onDismiss} title={title} footer={footer} className="eth-join">
      <div data-feature="join" data-stage={stage.type}>
        {body}
      </div>
    </Dialog>
  );
}
