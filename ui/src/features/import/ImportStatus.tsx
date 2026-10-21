import clsx from "clsx";
import { AlertTriangle, Import, X } from "lucide-react";
import { useState, type CSSProperties } from "react";
import { IconButton, Popover } from "@/kit";
import { StatusButton } from "@/features/status-bar/StatusButton";
import { Trans, useT, type MessageKey } from "@/i18n";
import { useImportStore, type ImportItem } from "./importStore";

const PHASE_LABEL: Record<ImportItem["phase"], MessageKey> = {
  queued: "import_audio.phase.queued",
  uploading: "import_audio.phase.uploading",
  decoding: "import_audio.phase.decoding",
  placing: "import_audio.phase.placing",
};

const percent = (p: number | null) => (p === null ? "…" : ` ${Math.round(p * 100)}%`);

function ImportRow({ item }: { item: ImportItem }) {
  const t = useT();
  const pct = item.progress === null ? null : Math.round(item.progress * 100);
  const failed = item.error !== null;
  return (
    <li className={clsx("eth-import__row", failed && "eth-import__row--error")} data-testid="import-row" data-phase={failed ? "error" : item.phase}>
      <span className="eth-import__text" title={item.error ?? item.name}>
        {failed ? (
          item.error
        ) : (
          <>
            <Trans k={PHASE_LABEL[item.phase]} values={{ name: <span className="eth-user-text">{item.name}</span> }} />
            {percent(item.progress)}
          </>
        )}
      </span>
      {!failed && (
        <span
          className={clsx("eth-import__bar", pct === null && "eth-import__bar--busy")}
          role="progressbar"
          aria-label={t("import_audio.importing_named", { name: item.name })}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct ?? undefined}
        >
          <span className="eth-import__fill" style={{ ["--eth-import-progress" as string]: pct === null ? "0" : String(pct / 100) }} />
        </span>
      )}
      {failed ? (
        <IconButton size="sm" tone="ghost" label={t("common.dismiss")} icon={<X />} onClick={() => useImportStore.getState().remove(item.id)} />
      ) : (
        item.cancel && <IconButton size="sm" tone="ghost" label={t("import_audio.cancel_named", { name: item.name })} icon={<X />} onClick={item.cancel} />
      )}
    </li>
  );
}

/** Imports in progress and failed ones (until dismissed), one row each. */
export function ImportList() {
  const t = useT();
  const items = useImportStore((s) => s.items);
  return (
    <ul className="eth-import" role="status" aria-label={t("import_audio.list")}>
      {items.map((i) => (
        <ImportRow key={i.id} item={i} />
      ))}
    </ul>
  );
}

/**
 * Status bar item while anything is imported or failed to import: the one file's phase (or
 * how many) and the progress; the list with cancel and dismiss in a popover above it.
 */
export function ImportStatus() {
  const t = useT();
  const items = useImportStore((s) => s.items);
  const [open, setOpen] = useState(false);
  if (items.length === 0) {
    if (open) setOpen(false);
    return null;
  }
  const active = items.filter((i) => i.error === null);
  const failed = items.length - active.length;
  const known = active.flatMap((i) => (i.progress === null ? [] : [i.progress]));
  const progress = known.length > 0 ? known.reduce((a, p) => a + p, 0) / known.length : null;
  const one = active.length === 1 ? active[0]! : null;
  let label;
  if (one) {
    label = (
      <>
        <Trans k={PHASE_LABEL[one.phase]} values={{ name: <span className="eth-user-text">{one.name}</span> }} />
        {percent(one.progress)}
      </>
    );
  } else if (active.length > 1) {
    label = t("import_audio.status.importing", { count: active.length }) + percent(progress);
  } else {
    label = t("import_audio.status.failed", { count: failed });
  }
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      placement="top-start"
      aria-label={t("import_audio.list")}
      className="eth-import-popover"
      trigger={(p) => (
        <StatusButton
          {...p}
          tone={failed > 0 ? "danger" : undefined}
          icon={active.length === 0 ? <AlertTriangle aria-hidden /> : <Import aria-hidden />}
          data-testid="import-status"
        >
          {label}
          {active.length > 0 && (
            <span className="eth-status-bar__progress" aria-hidden>
              <span
                className="eth-status-bar__progress-fill"
                style={{ "--eth-status-progress": progress === null ? "0" : String(progress) } as CSSProperties}
              />
            </span>
          )}
        </StatusButton>
      )}
    >
      <ImportList />
    </Popover>
  );
}
