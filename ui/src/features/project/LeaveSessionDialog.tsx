import { Button, Dialog } from "@/kit";
import type { EngineTransport } from "@/transport";
import { cancelLeave, confirmLeave, useLeaveGuard } from "./leaveGuard";

/** The confirmation dialog (mounted once, by the project menu). */
export function LeaveSessionDialog({ transport }: { transport: EngineTransport | null }) {
  const pending = useLeaveGuard((s) => s.pending);
  return (
    <Dialog
      open={pending !== null}
      onClose={cancelLeave}
      title={pending ? `Leave the “${pending.session}” session?` : ""}
      footer={
        <>
          <Button onClick={cancelLeave}>Cancel</Button>
          <Button tone="danger" onClick={() => void confirmLeave(transport)}>
            {pending?.confirm ?? "Leave & open"}
          </Button>
        </>
      }
    >
      <p className="eth-project-screen__hint">
        You stop collaborating on this session&apos;s project. Your copy stays in your projects: join the session again to keep working with the
        others.
      </p>
    </Dialog>
  );
}
