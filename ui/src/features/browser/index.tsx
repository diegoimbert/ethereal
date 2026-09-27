// OWNERSHIP: the `ui-shell` node owns `ui/src/features/browser/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) mounts `Browser`
// (keep the export name; its props are optional).
//
// Drop targets (arrangement): see `./dragPayload.ts` for the drag payload format
// and `resolveDroppedMedia` / `waitForMediaLength`.
import "./browser.css";
import clsx from "clsx";
import { useEffect, useRef, useState, type DragEvent, type KeyboardEvent, type ReactNode } from "react";
import { AudioLines, CornerLeftUp, File, Folder, FolderOpen, Music, Search, Volume2, X } from "lucide-react";
import type { BrowseLocation, BrowseRoot, DirectoryEntry, MediaSource } from "@/generated";
import { useUploadDrop } from "@/features/remote";
import { useEngineCommands, useEngineEvent } from "@/features/transport-bar/engine";
import { Button, TextInput } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { writeBrowserDrag, type BrowserDragPayload } from "./dragPayload";
import { formatSize, locationKey, parentPath, pathSegments, sameLocation, sourceOf } from "./paths";
import { useBrowserPreview } from "./preview";

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

/** Keyboard-navigable rows of a list. */
const ROW_SELECTOR = ".eth-browser__row[data-row]";

const KIND_ICON: Record<DirectoryEntry["kind"], ReactNode> = {
  Directory: <Folder />,
  Audio: <AudioLines />,
  Midi: <Music />,
  Other: <File />,
};

/**
 * Sample browser over the engine-visible locations (`Media::ListLocations`: library folders
 * and the current project's media). Folders navigate; clicking an audio file previews it
 * (click again to stop) and dragging it onto a drop target imports it there (payload:
 * `./dragPayload.ts`). The UI never accesses files itself.
 */
export function Browser({ scope = "all" }: { scope?: BrowserScope } = {}) {
  const { transport, send, error, clearError } = useEngineCommands();
  const hasProject = useProjectStore((s) => s.project !== null);
  const media = useProjectStore((s) => s.project?.media);
  const [locations, setLocations] = useState<BrowseRoot[] | null>(null);
  const [place, setPlace] = useState<Place | null>(null);
  const [listing, setListing] = useState<Listing | null>(null);
  const { previewing, toggle: togglePreview, play: playPreview } = useBrowserPreview(send);
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
    setQuery("");
  };

  const entries = listing && listing.key === currentKey ? listing.entries : null;

  // Keyboard folder navigation focuses the new folder's first row once it is listed.
  const focusFirstRow = useRef(false);
  const filesList = useRef<HTMLUListElement>(null);
  useEffect(() => {
    if (!focusFirstRow.current || !entries) return;
    focusFirstRow.current = false;
    filesList.current?.querySelector<HTMLElement>(ROW_SELECTOR)?.focus();
  }, [entries]);

  /**
   * Arrow keys walk the rows of a list (moving onto an audio file previews it), Home/End jump
   * to the ends; → opens a folder, ← / Backspace go up one.
   */
  const onListKeyDown = (e: KeyboardEvent<HTMLUListElement>, shown: DirectoryEntry[] | null | undefined) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>(ROW_SELECTOR);
    if (!row || !current) return;
    const entry = shown?.find((x) => x.path === row.dataset.path);
    if ((e.key === "ArrowLeft" || e.key === "Backspace") && current.path !== "") {
      e.preventDefault();
      focusFirstRow.current = true;
      navigate(parentPath(current.path));
      return;
    }
    if (e.key === "ArrowRight" && entry?.kind === "Directory") {
      e.preventDefault();
      focusFirstRow.current = true;
      navigate(entry.path);
      return;
    }
    const rows = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(ROW_SELECTOR));
    const i = rows.indexOf(row);
    const to =
      e.key === "ArrowDown" ? i + 1 : e.key === "ArrowUp" ? i - 1 : e.key === "Home" ? 0 : e.key === "End" ? rows.length - 1 : null;
    if (to === null) return;
    e.preventDefault();
    const next = rows[Math.max(0, Math.min(rows.length - 1, to))];
    if (!next || next === row) return;
    next.focus();
    next.scrollIntoView?.({ block: "nearest" });
    const target = shown?.find((x) => x.path === next.dataset.path);
    if (target?.kind === "Audio") void playPreview(target, sourceOf(current.location, target, media));
  };
  const currentRoot = current ? locations?.find((l) => sameLocation(l.location, current.location)) : undefined;

  return (
    <div className="eth-browser" data-feature="browser" {...useUploadDrop()}>
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
        <ul
          className="eth-browser__list"
          aria-label="Search results"
          aria-busy={!results?.done}
          onKeyDown={(e) => onListKeyDown(e, results?.entries)}
        >
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
                onPreview={togglePreview}
              />
            ))}
        </ul>
      ) : (
      <ul
        ref={filesList}
        className="eth-browser__list"
        aria-label="Files"
        aria-busy={current !== null && entries === null}
        onKeyDown={(e) => onListKeyDown(e, entries)}
      >
        {current && current.path !== "" && (
          <li>
            <div
              className="eth-browser__row eth-browser__row--dir"
              role="button"
              tabIndex={0}
              data-row
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
              onPreview={togglePreview}
            />
          ))}
      </ul>
      )}

      {error && (
        <button type="button" className="eth-browser__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
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
  onPreview(entry: DirectoryEntry, source: MediaSource): void;
}

function EntryRow({ entry, detail, source, previewing, onOpen, onPreview }: EntryRowProps) {
  const isDir = entry.kind === "Directory";
  const isAudio = entry.kind === "Audio";

  const activate = () => {
    if (isDir) onOpen();
    else if (isAudio) onPreview(entry, source);
  };

  const onDragStart = (e: DragEvent<HTMLDivElement>) => {
    const payload: BrowserDragPayload = { version: 1, kind: "media", source, name: entry.name, file_kind: entry.kind };
    writeBrowserDrag(e.dataTransfer, payload);
  };

  return (
    <li>
      <div
        className={clsx("eth-browser__row", `eth-browser__row--${entry.kind.toLowerCase()}`, previewing && "eth-browser__row--previewing")}
        role={isDir || isAudio ? "button" : undefined}
        tabIndex={isDir || isAudio ? 0 : undefined}
        aria-label={entry.name}
        aria-pressed={isAudio ? previewing : undefined}
        data-row={isDir || isAudio ? "" : undefined}
        data-path={entry.path}
        title={isAudio ? `${entry.name}: click to preview, drag to a track` : entry.name}
        draggable={isAudio}
        onDragStart={isAudio ? onDragStart : undefined}
        onClick={
          isDir || isAudio
            ? (e) => {
                e.currentTarget.focus();
                activate();
              }
            : undefined
        }
        onKeyDown={(e: KeyboardEvent) => {
          if (e.key === "Enter" && e.target === e.currentTarget) activate();
        }}
      >
        <span className="eth-browser__icon" aria-hidden>
          {previewing ? <Volume2 /> : KIND_ICON[entry.kind]}
        </span>
        <span className="eth-browser__name">{entry.name}</span>
        {detail !== undefined && detail !== "" && <span className="eth-browser__detail">{detail}</span>}
        {isAudio && <span className="eth-browser__size">{formatSize(entry.size)}</span>}
      </div>
    </li>
  );
}
