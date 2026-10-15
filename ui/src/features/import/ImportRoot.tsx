import { isEtherFileName } from "@/features/project/actions";
import "./import.css";
import clsx from "clsx";
import { X } from "lucide-react";
import { useContext, useEffect } from "react";
import { IconButton } from "@/kit";
import { TransportContext, type EngineTransport } from "@/transport";
import { importMidiFile, isMidiName } from "@/features/midi-file/importMidi";
import { importAudio } from "./importAudio";
import { useImportStore, type ImportItem } from "./importStore";
import { deliverPathDrop, isPathDropHost } from "./osDrop";
import { isImportShortcut, openImportDialog } from "./picker";
import { pathSource } from "./sources";
import { Trans, useT, type MessageKey } from "@/i18n";

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

const PHASE_LABEL: Record<ImportItem["phase"], MessageKey> = {
  queued: "import_audio.phase.queued",
  uploading: "import_audio.phase.uploading",
  decoding: "import_audio.phase.decoding",
  placing: "import_audio.phase.placing",
};

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
            {pct === null ? "…" : ` ${pct}%`}
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

/** Imports in progress and failed ones (until dismissed). */
export function ImportStatus() {
  const t = useT();
  const items = useImportStore((s) => s.items);
  if (items.length === 0) return null;
  return (
    <ul className="eth-import" role="status" aria-label={t("import_audio.list")} data-testid="import-status">
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
    return transport.onPathDrop((all) => {
      // An Ethereal project file is opened, not imported as audio.
      const paths = all.filter((p) => !isEtherFileName(p));
      if (paths.length === 0) return;
      if (deliverPathDrop(paths)) return;
      // Nowhere in particular: MIDI files become new MIDI tracks at the start (#717), audio
      // files are added to the project.
      const midi = paths.filter(isMidiName);
      for (const p of midi) importMidiFile(transport, pathSource(p), { track: null, at: 0 }).catch((e: unknown) => console.warn("MIDI file import failed", e));
      const audio = paths.filter((p) => !isMidiName(p));
      if (audio.length > 0) void importAudio(transport, audio.map(pathSource));
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
