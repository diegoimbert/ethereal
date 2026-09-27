import { Button, Dialog } from "@/kit";
import { ungroupTrack, useUngroupConfirm } from "./actions";

/** "Ungroup anyway?" when ungrouping would lose the group's devices, lanes or routings. */
export function UngroupConfirmDialog() {
  const pending = useUngroupConfirm((s) => s.pending);
  const transport = useUngroupConfirm((s) => s.transport);
  const close = useUngroupConfirm((s) => s.close);
  const confirm = () => {
    if (pending && transport) void ungroupTrack(transport, pending.group, true);
    close();
  };
  return (
    <Dialog
      open={pending !== null}
      onClose={close}
      title={pending ? `Ungroup “${pending.name}”?` : "Ungroup"}
      className="eth-groups-confirm"
      footer={
        <>
          <Button tone="ghost" onClick={close}>
            Cancel
          </Button>
          <Button tone="danger" onClick={confirm} autoFocus>
            Ungroup
          </Button>
        </>
      }
    >
      {pending && (
        <p className="eth-groups-confirm__text">
          The group track goes away with its {pending.losses.join(", ")}. Its tracks stay where they are. You can undo this.
        </p>
      )}
    </Dialog>
  );
}
