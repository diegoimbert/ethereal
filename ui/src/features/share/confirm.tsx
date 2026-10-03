// Confirmations asked from the Share popover ("Stop sharing", "Reset link"). The dialog is
// mounted by ShareControl, so it outlives the popover that asked.
import { Button, Dialog } from "@/kit";
import { useConfirm } from "./confirmStore";

export function ShareConfirmDialog() {
  const request = useConfirm((s) => s.request);
  const close = useConfirm((s) => s.close);
  return (
    <Dialog
      open={request !== null}
      onClose={close}
      title={request?.title ?? ""}
      footer={
        <>
          <Button onClick={close}>Cancel</Button>
          <Button
            tone={request?.danger ? "danger" : "accent"}
            autoFocus
            onClick={() => {
              close();
              request?.run();
            }}
          >
            {request?.action}
          </Button>
        </>
      }
    >
      <p className="eth-share-confirm">{request?.body}</p>
    </Dialog>
  );
}
