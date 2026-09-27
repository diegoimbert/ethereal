// OWNERSHIP: the `ui-shell` node owns `ui/src/features/browser/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) mounts `Browser`
// (keep the export name; its props are optional).
//
// Drop targets (arrangement): see `./dragPayload.ts` for the drag payload format
// and `resolveDroppedMedia` / `waitForMediaLength`.
import "./browser.css";
import clsx from "clsx";
import { useEffect, useState, type DragEvent, type KeyboardEvent, type ReactNode } from "react";
import { AudioLines, CornerLeftUp, File, Folder, FolderOpen, Music, Pause, Play, Plus, Search, X } from "lucide-react";
import type { BrowseLocation, BrowseRoot, DirectoryEntry, MediaSource } from "@/generated";
import { useEngineCommands, useEngineEvent } from "@/features/transport-bar/engine";
import { Button, TextInput } from "@/kit";
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

/** Which locations the browser shows: all of them, the library folders, or the project's media. */
export type BrowserScope = "all" | "library" | "project";

const inScope = (scope: BrowserScope) => (root: BrowseRoot) =>
  scope === "all" || (scope === "library" ? root.location.type === "Library" : root.location.type === "ProjectMedia");

/** Recursive search bounds (the engine lists one folder per request). */
const SEARCH_MAX_DIRS = 200;
const SEARCH_MAX_RESULTS = 300;
const SEARCH_DEBOUNCE_MS = 150;

interface Found {
  key: string;
  entries: DirectoryEntry[];
  done: boolean;
}

const KIND_ICON: Record<DirectoryEntry["kind"], ReactNode> = {
  Directory: <Folder />,
  Audio: <AudioLines />,
  Midi: <Music />,
  Other: <File />,
};

/**
 * Sample browser over the engine-visible locations (`Media::ListLocations`: library folders
 * and the current project's media). Folders navigate; audio files can be previewed,
 * imported into the project (double-click, Enter or "+") and dragged onto drop targets
 * (payload: `./dragPayload.ts`). The UI never accesses files itself.
 */
export function Browser({ scope = "all" }: { scope?: BrowserScope } = {}) {
  const { transport, send, error, clearError } = useEngineCommands();
  const hasProject = useProjectStore((s) => s.project !== null);
  const media = useProjectStore((s) => s.project?.media);
  const [locations, setLocations] = useState<BrowseRoot[] | null>(null);
  const [place, setPlace] = useState<Place | null>(null);
  const [listing, setListing] = useState<Listing | null>(null);
  const [previewing, setPreviewing] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<Found | null>(null);

  const ready = !!transport && hasProject;

  // Browse roots, once connected.
  useEffect(() => {
    if (!ready) return;
    let active = true;
    void send(cmd("Media", { type: "ListLocations" })).then((reply) => {
      if (active && reply?.type === "Locations") setLocations(reply.locations.filter(inScope(scope)));
    });
    return () => {
      active = false;
    };
  }, [ready, send, scope]);

  useEngineEvent((event) => {
    if (event.type === "Media" && event.event.type === "LocationsChanged") setLocations(event.event.locations.filter(inScope(scope)));
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

  // Search: every file and folder under the current one whose name contains the query
  // (walked folder by folder, bounded), refreshed as results come in.
  const q = query.trim().toLowerCase();
  const searchKey = current && q ? `${placeKey(current)}?${q}` : null;
  useEffect(() => {
    if (!ready || !current || !q) return;
    let active = true;
    const key = `${placeKey(current)}?${q}`;
    const { location, path } = current;
    const timer = setTimeout(async () => {
      const out: DirectoryEntry[] = [];
      const queue = [path];
      let dirs = 0;
      while (queue.length && active && dirs < SEARCH_MAX_DIRS && out.length < SEARCH_MAX_RESULTS) {
        dirs++;
        const reply = await send(cmd("Media", { type: "ListDirectory", location, path: queue.shift()! }));
        if (!active || reply?.type !== "Directory") continue;
        for (const e of reply.listing.entries) {
          if (e.kind === "Directory") queue.push(e.path);
          if (e.name.toLowerCase().includes(q)) out.push(e);
        }
        setFound({ key, entries: [...out], done: false });
      }
      if (active) setFound({ key, entries: out.slice(0, SEARCH_MAX_RESULTS), done: true });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      active = false;
      clearTimeout(timer);
    };
    // `current` is identified by `currentKey`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, send, currentKey, q]);
  const results = searchKey && found?.key === searchKey ? found : null;

  const navigate = (path: string) => {
    if (!current) return;
    setPlace({ location: current.location, path });
    setMessage(null);
    setQuery("");
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
    <div className="eth-browser" data-feature="browser">
      <div className="eth-browser__search">
        <Search className="eth-browser__search-icon" aria-hidden />
        <TextInput
          size="sm"
          type="search"
          className="eth-browser__search-input"
          aria-label="Search files"
          placeholder={currentRoot ? `Search ${currentRoot.name}` : "Search"}
          value={query}
          disabled={!current}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape" && query) {
              e.stopPropagation();
              setQuery("");
            }
          }}
        />
        {query && (
          <button type="button" className="eth-browser__search-clear" aria-label="Clear search" onClick={() => setQuery("")}>
            <X aria-hidden />
          </button>
        )}
      </div>
      <div
        className={clsx("eth-browser__locations", (locations?.length ?? 0) <= 1 && ready && "eth-browser__locations--single")}
        role="tablist"
        aria-label="Locations"
      >
        {(locations ?? []).map((root) => {
          const selected = !!current && sameLocation(root.location, current.location);
          return (
            <Button
              key={locationKey(root.location)}
              size="sm"
              tone="ghost"
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

      {q ? (
        <ul className="eth-browser__list" aria-label="Search results" aria-busy={!results?.done}>
          {results?.done && results.entries.length === 0 && (
            <li className="eth-browser__empty">
              <Search aria-hidden />
              <span>No matches</span>
              <span className="eth-browser__empty-hint">Nothing named like “{query.trim()}” in this folder or below.</span>
            </li>
          )}
          {current &&
            results?.entries.map((entry) => (
              <EntryRow
                key={entry.path}
                entry={entry}
                detail={parentPath(entry.path)}
                source={sourceOf(current.location, entry, media)}
                previewing={previewing === entry.path}
                onOpen={() => navigate(entry.path)}
                onImport={importEntry}
                onPreview={togglePreview}
              />
            ))}
        </ul>
      ) : (
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
              <span className="eth-browser__icon" aria-hidden>
                <CornerLeftUp />
              </span>
              <span className="eth-browser__name">..</span>
            </div>
          </li>
        )}
        {entries?.length === 0 && (
          <li className="eth-browser__empty">
            <FolderOpen aria-hidden />
            <span>Empty folder</span>
            <span className="eth-browser__empty-hint">Audio files you add here show up to drag onto tracks.</span>
          </li>
        )}
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
      )}

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
  /** Secondary text after the name (search results: the containing folder). */
  detail?: string;
  source: MediaSource;
  previewing: boolean;
  onOpen(): void;
  onImport(entry: DirectoryEntry, source: MediaSource): void;
  onPreview(entry: DirectoryEntry, source: MediaSource): void;
}

function EntryRow({ entry, detail, source, previewing, onOpen, onImport, onPreview }: EntryRowProps) {
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
        {detail !== undefined && detail !== "" && <span className="eth-browser__detail">{detail}</span>}
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
              {previewing ? <Pause aria-hidden /> : <Play aria-hidden />}
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
                <Plus aria-hidden />
              </Button>
            )}
          </span>
        )}
      </div>
    </li>
  );
}
