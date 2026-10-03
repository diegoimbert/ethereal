// The top bar's session element (docs/SHARING.md §8.1): a "Share" button, or the session pill
// with the Share popover. It also keeps the collaboration runtime mounted (presence, listen,
// chat toasts, the relay dialog of Settings > Advanced) and shows the view-only badge, the
// host-offline banner and the sharing toasts.
import { Share2 } from "lucide-react";
import { useContext, useEffect, useRef } from "react";
import { Badge, Button, Popover, Toast, ToastStack } from "@/kit";
import { cmd, TransportContext, type EngineTransport } from "@/transport";
import { CollabRuntime } from "@/features/collab";
import { useCollabStore } from "@/features/collab/store";
import { ShareConfirmDialog } from "./confirm";
import { SessionPill } from "./SessionPill";
import { othersOnline, sessionStatus } from "./status";
import { attempt } from "./actions";
import { SharePopoverContent } from "./SharePopover";
import { pushShareSettings, useShareSettings } from "./settings";
import { hostOf, participantsOf, useShareStore, useViewOnly } from "./store";
import { SHARE_TOAST_MS, useShareToasts } from "./toasts";
import "./share.css";

export function ShareControl() {
  const ctx = useContext(TransportContext);
  // Outside a TransportProvider (shell tests) the slot stays empty.
  if (!ctx) return <div className="eth-share" data-feature="share" />;
  return <ShareControlWith transport={ctx.transport} />;
}

/** Floating layers the popover must not take for an outside click (its own menus, selects, dialogs). */
const LAYERS = ".eth-popover, .eth-dialog-backdrop";

function ShareControlWith({ transport }: { transport: EngineTransport }) {
  const state = useShareStore((s) => s.state);
  const open = useShareStore((s) => s.popoverOpen);
  // A relay session (Settings > Advanced) shows its own bar (the collab runtime) instead.
  const inRelay = useCollabStore((s) => s.status.type !== "Offline");
  const viewOnly = useViewOnly();
  useShareEngine(transport);

  // A press in another floating layer (a row menu, the role list, a confirm dialog: all
  // portaled outside the popover) is not an outside click.
  const lastDown = useRef<Element | null>(null);
  useEffect(() => {
    const onDown = (e: PointerEvent) => (lastDown.current = e.target instanceof Element ? e.target : null);
    document.addEventListener("pointerdown", onDown, true);
    return () => document.removeEventListener("pointerdown", onDown, true);
  }, []);
  const setOpen = (o: boolean) => {
    if (!o && lastDown.current?.closest(LAYERS)) {
      lastDown.current = null;
      return;
    }
    // The join screen may have saved a name since.
    if (o) useShareSettings.getState().reload();
    useShareStore.getState().setPopoverOpen(o);
  };

  const status = sessionStatus(state);
  const people = othersOnline(participantsOf(state));
  const hidden = state.type === "Off" && inRelay;

  const share = async () => {
    if (await attempt(transport, cmd("Share", { type: "Start" }), "Couldn't share this project")) setOpen(true);
  };

  return (
    <div className="eth-share" data-feature="share" data-state={state.type}>
      <CollabRuntime shared={state.type !== "Off"} />
      {!hidden && (
        <Popover
          open={open}
          onOpenChange={setOpen}
          placement="bottom-end"
          aria-label="Share"
          className="eth-share-popover"
          trigger={(t) =>
            status ? (
              <SessionPill {...t} status={status} people={people} />
            ) : (
              <Button
                size="sm"
                tone="accent"
                aria-haspopup="dialog"
                aria-expanded={false}
                data-testid="share-button"
                title="Share this project: invite people with a link"
                onClick={() => void share()}
              >
                <Share2 className="eth-share__icon" aria-hidden />
                Share
              </Button>
            )
          }
        >
          {(close) => <SharePopoverContent transport={transport} state={state} close={close} />}
        </Popover>
      )}
      {viewOnly && (
        <span className="eth-share__view-only" title="You joined with a listen link: you can listen and chat, not edit" data-testid="view-only">
          <Badge tone="warn">View only</Badge>
        </span>
      )}
      <OfflineBanner />
      <ShareToasts />
      <ShareConfirmDialog />
    </div>
  );
}

/** Mirror `Event::Share`, ask for the current state, and push the identity and servers. */
function useShareEngine(transport: EngineTransport) {
  const onEvent = useShareStore((s) => s.onEvent);
  useEffect(() => transport.onEvent(onEvent), [transport, onEvent]);
  useEffect(() => {
    useShareStore.getState().reset();
    pushShareSettings(transport);
    void transport.send(cmd("Share", { type: "Get" })).catch(() => undefined);
  }, [transport]);
}

/** Joined while the host is offline: a thin banner under the top bar (§8.4). */
function OfflineBanner() {
  const state = useShareStore((s) => s.state);
  if (state.type !== "Joined" || state.link.type !== "HostOffline") return null;
  const host = hostOf(state)?.name || "The host";
  return (
    <div className="eth-share-banner" role="status" data-testid="share-offline-banner">
      <strong>{host} is offline.</strong> You're working on an offline copy. Changes sync when they're back; keep Ethereal open.
    </div>
  );
}

function ShareToasts() {
  const toasts = useShareToasts((s) => s.toasts);
  const dismiss = useShareToasts((s) => s.dismiss);
  if (toasts.length === 0) return null;
  return (
    <ToastStack label="Sharing">
      {toasts.map((t) => (
        <Toast key={t.id} title={t.title} accent={t.accent} timeoutMs={SHARE_TOAST_MS} onDismiss={() => dismiss(t.id)}>
          {t.text}
        </Toast>
      ))}
    </ToastStack>
  );
}
