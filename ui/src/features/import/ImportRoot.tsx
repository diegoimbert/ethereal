import "./import.css";
import clsx from "clsx";
import { X } from "lucide-react";
import { useContext, useEffect } from "react";
import { IconButton } from "@/kit";
import { TransportContext, type EngineTransport } from "@/transport";
import { importAudio } from "./importAudio";
import { useImportStore, type ImportItem } from "./importStore";
import { deliverPathDrop, isPathDropHost } from "./osDrop";
import { isImportShortcut, openImportDialog } from "./picker";
import { pathSource } from "./sources";

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

const PHASE_LABEL: Record<ImportItem["phase"], string> = {
  queued: "Waiting",
  uploading: "Uploading",
  decoding: "Loading",
  placing: "Adding",
};

function ImportRow({ item }: { item: ImportItem }) {
  const pct = item.progress === null ? null : Math.round(item.progress * 100);
  const failed = item.error !== null;
  return (
    <li className={clsx("eth-import__row", failed && "eth-import__row--error")} data-testid="import-row" data-phase={failed ? "error" : item.phase}>
      <span className="eth-import__text" title={item.error ?? item.name}>
        {failed ? item.error : `${PHASE_LABEL[item.phase]} ${item.name}${pct === null ? "…" : ` ${pct}%`}`}
      </span>
      {!failed && (
        <span
          className={clsx("eth-import__bar", pct === null && "eth-import__bar--busy")}
          role="progressbar"
          aria-label={`Importing ${item.name}`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct ?? undefined}
        >
          <span className="eth-import__fill" style={{ ["--eth-import-progress" as string]: pct === null ? "0" : String(pct / 100) }} />
        </span>
      )}
      {failed ? (
        <IconButton size="sm" tone="ghost" label="Dismiss" icon={<X />} onClick={() => useImportStore.getState().remove(item.id)} />
      ) : (
        item.cancel && <IconButton size="sm" tone="ghost" label={`Cancel ${item.name}`} icon={<X />} onClick={item.cancel} />
      )}
    </li>
  );
}

/** Imports in progress and failed ones (until dismissed). */
export function ImportStatus() {
  const items = useImportStore((s) => s.items);
  if (items.length === 0) return null;
  return (
    <ul className="eth-import" role="status" aria-label="Imports" data-testid="import-status">
      {items.map((i) => (
        <ImportRow key={i.id} item={i} />
      ))}
    </ul>
  );
}

function useImportCommands(transport: EngineTransport) {
  // ⌘I / Ctrl+I anywhere but in text fields.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !isImportShortcut(e) || isTextEntry(e.target)) return;
      e.preventDefault();
      void openImportDialog(transport);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [transport]);

  // Desktop: OS paths dropped on the window land in the zone hovered last; anywhere else
  // they are only added to the project.
  useEffect(() => {
    if (!isPathDropHost(transport)) return;
    return transport.onPathDrop((paths) => {
      if (!deliverPathDrop(paths)) void importAudio(transport, paths.map(pathSource));
    });
  }, [transport]);
}

function Commands({ transport }: { transport: EngineTransport }) {
  useImportCommands(transport);
  return null;
}

/**
 * Mounted once by the app shell: the ⌘I command, desktop path drops and the import status
 * list. Renders nothing without an engine connection.
 */
export function ImportRoot() {
  const transport = useContext(TransportContext)?.transport;
  return (
    <>
      {transport && <Commands transport={transport} />}
      <ImportStatus />
    </>
  );
}
