import { Button, Dialog } from "@/kit";
import { formatList, Trans, useT } from "@/i18n";
import { ungroupTrack, useUngroupConfirm } from "./actions";

/** "Ungroup anyway?" when ungrouping would lose the group's devices, lanes or routings. */
export function UngroupConfirmDialog() {
  const t = useT();
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
      title={pending ? <Trans k="groups.ungroup.title" values={{ name: <span className="eth-user-text">{pending.name}</span> }} /> : t("groups.menu.ungroup")}
      className="eth-groups-confirm"
      footer={
        <>
          <Button tone="ghost" onClick={close}>
            {t("groups.ungroup.cancel")}
          </Button>
          <Button tone="danger" onClick={confirm} autoFocus>
            {t("groups.menu.ungroup")}
          </Button>
        </>
      }
    >
      {pending && <p className="eth-groups-confirm__text">{t("groups.ungroup.text", { losses: formatList(pending.losses) })}</p>}
    </Dialog>
  );
}
