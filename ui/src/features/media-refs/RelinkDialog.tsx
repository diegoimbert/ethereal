import clsx from "clsx";
import { FolderSearch, FileAudio, Search } from "lucide-react";
import { useState } from "react";
import type { MediaId, MediaRef, MediaSource } from "@/generated";
import { Badge, Button, Dialog } from "@/kit";
import { useProjectStore } from "@/state";
import type { EngineTransport } from "@/transport";
import { closeRelink, collectAll, errorText, relink, search, useMediaRefs } from "./store";
import { hasFolderPicker, locationLabel, pickRelinkSource, sourceLabel } from "./sources";

function MediaRow({ media, transport, busy, setBusy }: { media: MediaRef; transport: EngineTransport; busy: boolean; setBusy: (b: boolean) => void }) {
  const missing = useMediaRefs((s) => s.missing.has(media.id));
  const candidates = useMediaRefs((s) => s.candidates[media.id]);
  const act = async (body: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await body();
    } catch (e) {
      useMediaRefs.setState({ error: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  const use = (source: MediaSource) => act(() => relink(transport, media.id, source));
  const locate = () =>
    act(async () => {
      const source = await pickRelinkSource(transport);
      if (source) await relink(transport, media.id, source);
    });
  return (
    <li className={clsx("eth-media-refs__row", !missing && "eth-media-refs__row--found")} data-testid="relink-row" data-media={media.id}>
      <FileAudio className="eth-media-refs__icon" aria-hidden />
      <div className="eth-media-refs__info">
        <span className="eth-media-refs__name">{media.name}</span>
        <span className="eth-media-refs__path" title={locationLabel(media)}>
          {locationLabel(media)}
        </span>
      </div>
      {missing ? <Badge tone="warn">Missing</Badge> : <Badge tone="ok">Found</Badge>}
      <Button size="sm" disabled={busy} onClick={() => void locate()}>
        Locate…
      </Button>
      {candidates && missing && (
        <div className="eth-media-refs__candidates">
          {candidates.length === 0 ? (
            <span className="eth-media-refs__hint">No file with this name was found.</span>
          ) : (
            <>
              <span className="eth-media-refs__hint">Same name, different content:</span>
              <ul>
                {candidates.map((c) => (
                  <li key={sourceLabel(c)} className="eth-media-refs__candidate">
                    <span className="eth-media-refs__path" title={sourceLabel(c)}>
                      {sourceLabel(c)}
                    </span>
                    <Button size="sm" tone="ghost" disabled={busy} onClick={() => void use(c)}>
                      Use
                    </Button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </li>
  );
}

/**
 * "Relink…": the missing samples of the project (or one of them), each with "Locate…" (the
 * OS file dialog on the desktop, the browser's file picker elsewhere), a library search
 * (files with the same content are relinked at once, same-name files are offered), a folder
 * search on the desktop, and "Collect All and Save" (copy every referenced sample into the
 * project folder, one undo step).
 */
export function RelinkDialog({ transport }: { transport: EngineTransport }) {
  const dialog = useMediaRefs((s) => s.dialog);
  const missing = useMediaRefs((s) => s.missing);
  const searchState = useMediaRefs((s) => s.search);
  const collect = useMediaRefs((s) => s.collect);
  const error = useMediaRefs((s) => s.error);
  const media = useProjectStore((s) => s.project?.media);
  const [busy, setBusy] = useState(false);
  const [shown, setShown] = useState<MediaId[]>([]);
  const open = dialog !== null;

  // Rows: the focused media, or every missing one (kept once found, until reopened).
  const ids = dialog?.media ? [dialog.media] : [...new Set([...shown, ...missing])];
  if (open && !dialog.media && ids.some((id) => !shown.includes(id))) setShown(ids);
  if (!open && shown.length > 0) setShown([]);
  const rows = ids.map((id) => media?.[id]).filter((m): m is MediaRef => !!m);
  const stillMissing = rows.filter((m) => missing.has(m.id)).length;
  const searching = searchState !== null && !searchState.done;
  const external = Object.values(media ?? {}).some((m) => m.location.type === "External");
  const folderPicker = hasFolderPicker(transport);

  const searchIn = async (folder?: string) => {
    setBusy(true);
    await search(transport, dialog?.media ?? null, folder);
    setBusy(false);
  };
  const searchFolder = async () => {
    if (!hasFolderPicker(transport)) return;
    const folder = await transport.pickFolder();
    if (folder) await searchIn(folder);
  };

  const title = dialog?.media ? `Relink “${rows[0]?.name ?? "sample"}”` : "Missing samples";
  return (
    <Dialog
      open={open}
      onClose={closeRelink}
      title={title}
      className="eth-media-refs"
      footer={
        <>
          <Button
            disabled={busy || collect !== null || !external}
            title="Copy every sample the project references into its folder, then save"
            onClick={() => void collectAll(transport)}
          >
            {collect ? `Collecting ${collect.done}/${collect.total}…` : "Collect All and Save"}
          </Button>
          <Button tone="accent" onClick={closeRelink}>
            Done
          </Button>
        </>
      }
    >
      <p className="eth-media-refs__summary" data-testid="relink-summary">
        {stillMissing === 0
          ? rows.length > 0
            ? "Every sample is linked."
            : "No sample is missing."
          : `${stillMissing} ${stillMissing === 1 ? "sample can't" : "samples can't"} be found. Clips that use ${stillMissing === 1 ? "it" : "them"} play silence until relinked.`}
      </p>
      {rows.length > 0 && (
        <ul className="eth-media-refs__list" aria-label="Samples">
          {rows.map((m) => (
            <MediaRow key={m.id} media={m} transport={transport} busy={busy} setBusy={setBusy} />
          ))}
        </ul>
      )}
      <div className="eth-media-refs__search">
        <Button size="sm" disabled={busy || searching || stillMissing === 0} onClick={() => void searchIn()}>
          <Search aria-hidden /> Search library
        </Button>
        {folderPicker && (
          <Button size="sm" disabled={busy || searching || stillMissing === 0} onClick={() => void searchFolder()}>
            <FolderSearch aria-hidden /> Search in folder…
          </Button>
        )}
        {searchState && (
          <span className="eth-media-refs__hint" role="status" data-testid="relink-search-status">
            {searching ? `Searching… ${searchState.scanned} folders` : `Searched ${searchState.scanned} folders`}
          </span>
        )}
      </div>
      {error && (
        <p className="eth-media-refs__error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
