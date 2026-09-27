// OWNERSHIP: the `ui-shell` node owns `ui/src/features/browser/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `Browser`: keep this export name and keep it prop-less (read state via hooks).
//
// Drop targets (arrangement): see `./dragPayload.ts` for the drag payload format
// and `resolveDroppedMedia` / `waitForMediaLength`.
import "./browser.css";
import clsx from "clsx";
import { useEffect, useState, type DragEvent, type KeyboardEvent } from "react";
import type { BrowseLocation, BrowseRoot, DirectoryEntry, MediaSource } from "@/generated";
import { useUploadDrop } from "@/features/remote";
import { useEngineCommands, useEngineEvent } from "@/features/transport-bar/engine";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, newId } from "@/transport";
import { writeBrowserDrag, type BrowserDragPayload } from "./dragPayload";
import { formatSize, locationKey, parentPath, pathSegments, sameLocation, sourceOf } from "./paths";

interface Place {
  location: BrowseLocation;
  path: string;
}

interface Listing {
  key: string;
  entries: DirectoryEntry[];
}

const placeKey = (p: Place) => `${locationKey(p.location)}/${p.path}`;

const KIND_ICON: Record<DirectoryEntry["kind"], string> = {
  Directory: "📁",
  Audio: "♪",
  Midi: "♫",
  Other: "·",
};

/**
 * Sample browser over the engine-visible locations (`Media::ListLocations`: library folders
 * and the current project's media). Folders navigate; audio files can be previewed,
 * imported into the project (double-click, Enter or "+") and dragged onto drop targets
 * (payload: `./dragPayload.ts`). The UI never accesses files itself.
 */
export function Browser() {
  const { transport, send, error, clearError } = useEngineCommands();
  const hasProject = useProjectStore((s) => s.project !== null);
  const media = useProjectStore((s) => s.project?.media);
  const [locations, setLocations] = useState<BrowseRoot[] | null>(null);
  const [place, setPlace] = useState<Place | null>(null);
  const [listing, setListing] = useState<Listing | null>(null);
  const [previewing, setPreviewing] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const ready = !!transport && hasProject;

  // Browse roots, once connected.
  useEffect(() => {
    if (!ready) return;
    let active = true;
    void send(cmd("Media", { type: "ListLocations" })).then((reply) => {
      if (active && reply?.type === "Locations") setLocations(reply.locations);
    });
    return () => {
      active = false;
    };
  }, [ready, send]);

  useEngineEvent((event) => {
    if (event.type === "Media" && event.event.type === "LocationsChanged") setLocations(event.event.locations);
  });

  // The shown place: the chosen one if its location still exists, else the first root.
  const current: Place | null =
    place && locations?.some((l) => sameLocation(l.location, place.location))
      ? place
      : locations?.[0]
        ? { location: locations[0].location, path: "" }
        : null;
  const currentKey = current ? placeKey(current) : null;
  // The project's media folder changes with the document: re-list it on media patches.
  const mediaDep = current?.location.type === "ProjectMedia" ? media : null;

  useEffect(() => {
    if (!ready || !current) return;
    let active = true;
    const key = placeKey(current);
    void send(cmd("Media", { type: "ListDirectory", location: current.location, path: current.path })).then((reply) => {
      if (active && reply?.type === "Directory") setListing({ key, entries: reply.listing.entries });
    });
    return () => {
      active = false;
    };
    // `current` is identified by `currentKey`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, send, currentKey, mediaDep]);

  const navigate = (path: string) => {
    if (!current) return;
    setPlace({ location: current.location, path });
    setMessage(null);
  };

  const importEntry = async (entry: DirectoryEntry, source: MediaSource) => {
    if (source.type === "Project") {
      setMessage(`${entry.name} is already in the project`);
      return;
    }
    const reply = await send(cmd("Media", { type: "Import", id: newId(), source }));
    if (reply?.type === "Media") setMessage(`Imported ${reply.media.name}`);
  };

  const togglePreview = async (entry: DirectoryEntry, source: MediaSource) => {
    if (previewing === entry.path) {
      setPreviewing(null);
      await send(cmd("Media", { type: "StopPreview" }));
      return;
    }
    const reply = await send(cmd("Media", { type: "Preview", source }));
    setPreviewing(reply ? entry.path : null);
  };

  const entries = listing && listing.key === currentKey ? listing.entries : null;
  const currentRoot = current ? locations?.find((l) => sameLocation(l.location, current.location)) : undefined;

  return (
    <div className="eth-browser" data-feature="browser" {...useUploadDrop()}>
      <div className="eth-browser__locations" role="tablist" aria-label="Locations">
        {(locations ?? []).map((root) => {
          const selected = !!current && sameLocation(root.location, current.location);
          return (
            <Button
              key={locationKey(root.location)}
              size="sm"
              role="tab"
              aria-selected={selected}
              active={selected}
              onClick={() => {
                setPlace({ location: root.location, path: "" });
                setMessage(null);
              }}
            >
              {root.name}
            </Button>
          );
        })}
        {!ready && <span className="eth-browser__hint">No engine connected</span>}
      </div>

      {current && (
        <nav className="eth-browser__crumbs" aria-label="Path">
          <button type="button" className="eth-browser__crumb" onClick={() => navigate("")}>
            {currentRoot?.name ?? "Root"}
          </button>
          {pathSegments(current.path).map((seg) => (
            <span key={seg.path}>
              <span className="eth-browser__sep">/</span>
              <button type="button" className="eth-browser__crumb" onClick={() => navigate(seg.path)}>
                {seg.name}
              </button>
            </span>
          ))}
        </nav>
      )}

      <ul className="eth-browser__list" aria-label="Files" aria-busy={current !== null && entries === null}>
        {current && current.path !== "" && (
          <li>
            <div
              className="eth-browser__row eth-browser__row--dir"
              role="button"
              tabIndex={0}
              aria-label="Parent folder"
              onClick={() => navigate(parentPath(current.path))}
              onKeyDown={(e) => e.key === "Enter" && navigate(parentPath(current.path))}
            >
              <span className="eth-browser__icon">↩</span>
              <span className="eth-browser__name">..</span>
            </div>
          </li>
        )}
        {entries?.length === 0 && <li className="eth-browser__hint">Empty folder</li>}
        {current &&
          entries?.map((entry) => (
            <EntryRow
              key={entry.path}
              entry={entry}
              source={sourceOf(current.location, entry, media)}
              previewing={previewing === entry.path}
              onOpen={() => navigate(entry.path)}
              onImport={importEntry}
              onPreview={togglePreview}
            />
          ))}
      </ul>

      {error ? (
        <button type="button" className="eth-browser__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
      ) : (
        message && (
          <div className="eth-browser__status" role="status">
            {message}
          </div>
        )
      )}
    </div>
  );
}

interface EntryRowProps {
  entry: DirectoryEntry;
  source: MediaSource;
  previewing: boolean;
  onOpen(): void;
  onImport(entry: DirectoryEntry, source: MediaSource): void;
  onPreview(entry: DirectoryEntry, source: MediaSource): void;
}

function EntryRow({ entry, source, previewing, onOpen, onImport, onPreview }: EntryRowProps) {
  const isDir = entry.kind === "Directory";
  const isAudio = entry.kind === "Audio";
  const inProject = source.type === "Project";

  const activate = () => {
    if (isDir) onOpen();
    else if (isAudio) onImport(entry, source);
  };

  const onDragStart = (e: DragEvent<HTMLDivElement>) => {
    const payload: BrowserDragPayload = { version: 1, kind: "media", source, name: entry.name, file_kind: entry.kind };
    writeBrowserDrag(e.dataTransfer, payload);
  };

  return (
    <li>
      <div
        className={clsx("eth-browser__row", `eth-browser__row--${entry.kind.toLowerCase()}`)}
        role={isDir || isAudio ? "button" : undefined}
        tabIndex={isDir || isAudio ? 0 : undefined}
        aria-label={entry.name}
        title={isAudio ? `${entry.name}: drag to a track, double-click to import` : entry.name}
        draggable={isAudio}
        onDragStart={isAudio ? onDragStart : undefined}
        onClick={isDir ? onOpen : undefined}
        onDoubleClick={isAudio ? activate : undefined}
        onKeyDown={(e: KeyboardEvent) => {
          if (e.key === "Enter" && e.target === e.currentTarget) activate();
        }}
      >
        <span className="eth-browser__icon" aria-hidden>
          {KIND_ICON[entry.kind]}
        </span>
        <span className="eth-browser__name">{entry.name}</span>
        {isAudio && (
          <span className="eth-browser__actions">
            <span className="eth-browser__size">{formatSize(entry.size)}</span>
            <Button
              size="sm"
              variant="ghost"
              aria-label={previewing ? `Stop preview of ${entry.name}` : `Preview ${entry.name}`}
              active={previewing}
              onClick={(e) => {
                e.stopPropagation();
                onPreview(entry, source);
              }}
            >
              {previewing ? "■" : "▶"}
            </Button>
            {!inProject && (
              <Button
                size="sm"
                variant="ghost"
                aria-label={`Import ${entry.name}`}
                title="Import into project"
                onClick={(e) => {
                  e.stopPropagation();
                  onImport(entry, source);
                }}
              >
                +
              </Button>
            )}
          </span>
        )}
      </div>
    </li>
  );
}
