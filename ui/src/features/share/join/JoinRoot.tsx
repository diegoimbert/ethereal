import { useCallback, useContext, useEffect, useRef, useState } from "react";
import type { InvitePreview, JoinStage, ProjectId, ShareCommand, ShareState } from "@/generated";
import { Toast, ToastStack } from "@/kit";
import { cmd, type EngineTransport, type Unsubscribe } from "@/transport";
import { isCommandFailed } from "@/transport/EngineTransport";
import { TransportContext } from "@/transport/context";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { loadIdentity, saveIdentity, type ShareIdentity } from "./identity";
import { JoinScreen } from "./JoinScreen";
import { JoinWithLinkDialog } from "./JoinWithLink";
import { inviteLinkFrom } from "./links";
import { openInvite, useJoinStore, type QueuedInvite } from "./store";

/** A transport that delivers `ethereal://` deep links (`TauriTransport`). */
interface DeepLinkSource {
  onDeepLink(listener: (url: string) => void): Unsubscribe;
}

const hasDeepLinks = (t: EngineTransport | null | undefined): t is EngineTransport & DeepLinkSource =>
  !!t && typeof (t as Partial<DeepLinkSource>).onDeepLink === "function";

const JOINED_TOAST_MS = 5000;

/** A UI-side failure shown on the join screen (the engine refused the command itself). */
function localFailure(e: unknown): JoinStage {
  let message: string;
  if (isCommandFailed(e, "InvalidState")) message = "Stop sharing your project before joining another one.";
  else if (isCommandFailed(e, "Unsupported")) message = "Joining shared projects isn't available in this version of Ethereal yet.";
  else message = e instanceof Error ? e.message : String(e);
  // Shown as is (`failureText` uses the message of a `BadLink` failure).
  return { type: "Failed", reason: "BadLink", message };
}

/**
 * The join flow (docs/SHARING.md §5, §8.3), mounted once by the app shell:
 * - sends queued invites (`openInvite`: the web `/join/` route, a pasted link) with
 *   `Share::OpenInvite` once the engine is connected;
 * - on desktop, routes `ethereal://join/...` deep links (cold and warm start) the same way;
 * - shows the `JoinScreen` while `ShareState` is `Joining`, then the "You're in …" toast;
 * - hosts the "Join with a link…" dialog (`openJoinWithLink()`).
 */
export function JoinRoot() {
  const ctx = useContext(TransportContext);
  const transport = ctx?.transport ?? null;
  const connected = ctx?.connection.status === "connected";
  // Deep links come from the desktop shell even while a remote engine is in use.
  const shell = useTransportSwitch()?.local ?? transport;

  const [share, setShare] = useState<ShareState>({ type: "Off" });
  const [local, setLocal] = useState<JoinStage | null>(null);
  const [hidden, setHidden] = useState(false);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [identity, setIdentity] = useState<ShareIdentity | null>(loadIdentity);
  const [invite, setInvite] = useState<InvitePreview | null>(null);
  const lastInvite = useRef<InvitePreview | null>(null);
  const queue = useJoinStore((s) => s.queue);

  const send = useCallback(
    (c: ShareCommand) => {
      if (!transport) return Promise.reject(new Error("no engine"));
      return transport.send(cmd("Share", c));
    },
    [transport],
  );

  // `ShareState` from the engine.
  const shareRef = useRef(share);
  useEffect(() => {
    if (!transport) return;
    return transport.onEvent((e) => {
      if (e.type !== "Share" || e.event.type !== "State") return;
      const prev = shareRef.current;
      const next = e.event.state;
      shareRef.current = next;
      if (prev.type === "Joining" && next.type === "Joined") {
        const inv = lastInvite.current;
        const host = next.participants.find((p) => p.role === "Host")?.name ?? inv?.host.name;
        const project = inv?.project_name ?? "the project";
        setToast(host ? `You're in ${project} with ${host}` : `You're in ${project}`);
      }
      if (next.type === "Joining" && next.stage.type === "Ready") {
        lastInvite.current = next.stage.invite;
        setInvite(next.stage.invite);
      }
      if (next.type !== "Joining" || next.stage.type !== "Ready") setBusy(false);
      // A new invite (or a new stage of one) shows the screen again.
      if (next.type === "Joining" && (prev.type !== "Joining" || prev.stage.type !== next.stage.type)) setHidden(false);
      setShare(next);
    });
  }, [transport]);

  useEffect(() => {
    if (connected) send({ type: "Get" }).catch(() => {});
  }, [connected, send]);

  // Queued invites → `OpenInvite`.
  useEffect(() => {
    if (!connected || !transport || queue.length === 0) return;
    for (const { link, onDelivered } of useJoinStore.getState().take() as QueuedInvite[]) {
      send({ type: "OpenInvite", link })
        .then(
          () => setLocal(null),
          (e: unknown) => {
            setLocal(localFailure(e));
            setHidden(false);
          },
        )
        .finally(() => onDelivered?.());
    }
  }, [connected, transport, queue, send]);

  // Desktop deep links.
  useEffect(() => {
    if (!hasDeepLinks(shell)) return;
    return shell.onDeepLink((url) => {
      const link = inviteLinkFrom(url);
      if (link) openInvite(link);
      else console.warn("[ethereal] ignored a deep link that is not an invite");
    });
  }, [shell]);

  const joining = share.type === "Joining" ? share.stage : null;
  const stage = local ?? joining;
  // base-131: the project screen (shown on launch, with no project open) steps aside.
  const screenShown = stage !== null && !hidden;
  useEffect(() => {
    useJoinStore.setState({ screenShown });
  }, [screenShown]);
  useEffect(() => () => useJoinStore.setState({ screenShown: false }), []);

  // Kept while the dialog animates out.
  const [shown, setShown] = useState<JoinStage | null>(stage);
  if (stage && stage !== shown) setShown(stage);

  const dismiss = () => {
    setHidden(true);
    setLocal(null);
    setBusy(false);
    if (share.type === "Joining") send({ type: "Leave" }).catch(() => {});
  };

  const join = async (name: string | null) => {
    setBusy(true);
    try {
      let who = identity;
      if (name !== null) {
        who = { name, color: identity?.color ?? null };
        saveIdentity(who);
        setIdentity(who);
      }
      if (who) await send({ type: "SetIdentity", name: who.name, color: who.color });
      await send({ type: "AcceptInvite" });
    } catch (e) {
      setBusy(false);
      setLocal(localFailure(e));
    }
  };

  const openOfflineCopy = (project: ProjectId) => {
    setHidden(true);
    void send({ type: "Leave" })
      .catch(() => {})
      .then(() => transport?.send(cmd("Project", { type: "Open", id: project })))
      .catch((e: unknown) => setLocal(localFailure(e)));
  };

  return (
    <>
      {shown && (
        <JoinScreen
          open={stage !== null && !hidden}
          stage={shown}
          invite={invite}
          askName={identity === null}
          busy={busy}
          onJoin={(n) => void join(n)}
          onDismiss={dismiss}
          onOpenOfflineCopy={openOfflineCopy}
        />
      )}
      <JoinWithLinkDialog />
      {toast && (
        <ToastStack label="Join notifications">
          <Toast title={toast} timeoutMs={JOINED_TOAST_MS} onDismiss={() => setToast(null)} />
        </ToastStack>
      )}
    </>
  );
}
