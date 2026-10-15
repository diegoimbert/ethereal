// The generic confirmation dialog: destructive or hard-to-undo actions ask through
// `useConfirm.getState().ask(...)` (`./confirmStore`), and `<ConfirmDialog />` (mounted once
// by the shell) renders the pending request.
import { useT } from "@/i18n";
import { Button } from "./Button";
import { useConfirm } from "./confirmStore";
import { Dialog } from "./overlays";

export function ConfirmDialog() {
  const request = useConfirm((s) => s.request);
  const close = useConfirm((s) => s.close);
  const t = useT();
  return (
    <Dialog
      open={request !== null}
      onClose={close}
      title={request?.title ?? ""}
      footer={
        <>
          <Button onClick={close}>{t("common.cancel")}</Button>
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
      <p className="eth-confirm">{request?.body}</p>
    </Dialog>
  );
}
