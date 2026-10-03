/**
 * The indexed library browser (v0.2 `browser-v2`, CONTRACTS.md §12.8), in the folder
 * browser's visual language (`../index.tsx`, `../browser.css`):
 * - search over the engine index (`Browser::Query`, debounced, by relevance while typing),
 *   kind chips, favourites, sort and tempo-synced preview ("Sync", remembered);
 * - places: "All", the library roots, sample packs, user folders and factory presets,
 *   then "Add folder…" (desktop: the OS folder dialog; the folder is indexed in place) or
 *   "Import folder…" (web, remote engine: the folder's audio is copied into the engine's
 *   library, `../folders`); OS folders dropped on the browser are added the same way
 *   (desktop paths) or imported; user folders can be renamed and removed from their
 *   context menu; a place without filters browses folder by folder exactly like
 *   the folder browser (`Media::ListDirectory`), filters turn it into index results scoped
 *   to the place and folder;
 * - result rows preview (click, Enter, arrow keys), drag (the `../dragPayload.ts` payload),
 *   star, show tags (click: filter by it); their context menu edits favourites and tags,
 *   shows the item's folder, loads presets on the selected track's device, opens projects;
 * - paged (100 at a time, more on scroll), re-queried on `IndexChanged`; `IndexProgress`
 *   shows a status line; saved presets trigger a rescan of the User Library.
 */
import "../browser.css";
import "./browserV2.css";
import clsx from "clsx";
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent, type MouseEvent } from "react";
import { CornerLeftUp, FolderOpen, FolderPlus, Search, Star, X } from "lucide-react";
import type { BrowseRoot, BrowserQuery, BrowserRoot, DirectoryEntry, LibraryItem } from "@/generated";
import { BrowserImportBar, fileSource, hasOsFiles, importAudio, isPathDropHost, noteHover, useImportDrop, type ImportSource } from "@/features/import";
import { isAudioName } from "@/features/import/sources";
import { useCurrentPresets } from "@/features/presets";
import { useEngineCommands, useEngineEvent } from "@/features/transport-bar/engine";
import { Button, Dialog, IconButton, Select, TextInput, openContextMenu, type ContextMenuEntry } from "@/kit";
import { devicesOfTrack, useProjectStore, useSelectionStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { EntryRow, ROW_SELECTOR, SEARCH_DEBOUNCE_MS, type BrowserScope } from "../index";
import { FolderImportStatus, droppedItems, importFolder, pickFolder, useFolderImports, type PickedFolder } from "../folders";
import { parentPath, pathSegments, sourceOf } from "../paths";
import { ItemRow } from "./ItemRow";
import {
  ALL_ROOTS,
  KIND_FILTERS,
  PAGE_SIZE,
  PROJECT_MEDIA,
  SORTS,
  buildQuery,
  builtinKey,
  filtersActive,
  joinPath,
  loadSync,
  matchingFolders,
  parseTags,
  placeOfFolder,
  placeOfItem,
  presetDeviceKey,
  rootLabel,
  rootLocation,
  saveSync,
  type ChosenSort,
  type KindFilter,
  type Place,
} from "./model";
import { useItemPreview, type PreviewTarget } from "./preview";

/** The query limit's upper bound (the engine clamps). */
const MAX_LIMIT = 200;
/** Load the next page when the list is scrolled this close to its end (px). */
const LOAD_MORE_MARGIN = 240;

interface Results {
  /** The query (JSON, first page) these results answer. */
  query: string;
  items: LibraryItem[];
  total: number;
}

interface Listing {
  key: string;
  entries: DirectoryEntry[];
}

/** The desktop transport's folder dialog (`TauriTransport.pickFolder`), duck-typed. */
const folderPicker = (t: EngineTransport | null) =>
  typeof (t as { pickFolder?: unknown } | null)?.pickFolder === "function"
    ? (t as unknown as { pickFolder(): Promise<string | null> })
    : null;

export interface BrowserV2Props {
  scope: BrowserScope;
  /** `Browser::ListRoots` (the probe's reply). */
  initialRoots: BrowserRoot[];
}

export function BrowserV2({ scope, initialRoots }: BrowserV2Props) {
  const { transport, send, error, clearError } = useEngineCommands();
  const [ownError, setOwnError] = useState<string | null>(null);
  const media = useProjectStore((s) => s.project?.media);
  const [roots, setRoots] = useState(initialRoots);
  const [locations, setLocations] = useState<BrowseRoot[] | null>(null);
  const [place, setPlace] = useState<Place | null>(null);
  const [text, setText] = useState("");
  const [typed, setDebounced] = useState("");
  const [kind, setKind] = useState<KindFilter>("all");
  const [favouritesOnly, setFavouritesOnly] = useState(false);
  const [sort, setSort] = useState<ChosenSort>("Name");
  const [tags, setTags] = useState<string[]>([]);
  const [sync, setSync] = useState(loadSync);
  const [refresh, setRefresh] = useState(0);
  const [indexing, setIndexing] = useState<{ root: string; scanned: number } | null>(null);
  const [results, setResults] = useState<Results | null>(null);
  const [listing, setListing] = useState<Listing | null>(null);
  const [editing, setEditing] = useState<LibraryItem | null>(null);
  const [renaming, setRenaming] = useState<BrowserRoot | null>(null);
  const [dropOnPlaces, setDropOnPlaces] = useState(false);
  const preview = useItemPreview(transport, send, sync, setOwnError);
  const shownError = ownError ?? error;

  const listRoots = useCallback(() => {
    void send(cmd("Browser", { type: "ListRoots" })).then((r) => {
      if (r?.type === "BrowserRoots") setRoots(r.roots);
    });
  }, [send]);

  // Browse locations (folder view of library roots; project media in the "all" scope).
  const scoped = useCallback(
    (ls: BrowseRoot[]) => ls.filter((l) => l.location.type === "Library" || scope === "all"),
    [scope],
  );
  useEffect(() => {
    let active = true;
    void send(cmd("Media", { type: "ListLocations" })).then((r) => {
      if (active) setLocations(r?.type === "Locations" ? scoped(r.locations) : []);
    });
    return () => {
      active = false;
    };
  }, [send, scoped]);

  useEngineEvent((event) => {
    if (event.type === "Media" && event.event.type === "LocationsChanged") setLocations(scoped(event.event.locations));
    if (event.type === "Browser") {
      if (event.event.type === "IndexProgress") setIndexing({ root: event.event.root, scanned: event.event.scanned });
      else {
        setIndexing(null);
        setRefresh((n) => n + 1);
        listRoots();
      }
    }
    // Presets saved, renamed or deleted: keep the User Library's index fresh.
    if (event.type === "Preset" && event.event.type === "Changed")
      transport?.send(cmd("Browser", { type: "Rescan", root: "user" })).catch(() => undefined);
  });

  const hasProjectMedia = scope === "all" && !!locations?.some((l) => l.location.type === "ProjectMedia");
  const validRoot = (r: string) => r === ALL_ROOTS || (r === PROJECT_MEDIA ? hasProjectMedia : roots.some((x) => x.id === r));
  // The shown place: the chosen one if it still exists, else the first browsable root.
  const current: Place | null =
    locations === null
      ? null
      : place && validRoot(place.root)
        ? place
        : { root: roots.find((r) => rootLocation(r.id, roots, locations))?.id ?? ALL_ROOTS, folder: "" };
  const loc = current ? rootLocation(current.root, roots, locations) : null;
  const filters = { text, kind, tags, favouritesOnly, sort };
  const folderMode = !!current && !!loc && (current.root === PROJECT_MEDIA || !filtersActive(filters));
  const indexMode = !!current && !folderMode;

  // ---- Index results ---------------------------------------------------------------------
  const debounced = text.trim() ? typed : "";
  useEffect(() => {
    if (!text.trim()) return;
    const t = setTimeout(() => setDebounced(text), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [text]);

  // While typing, wait for the debounced text (no flash of unfiltered results).
  const pending = text.trim() !== debounced.trim();
  const query: BrowserQuery | null = indexMode && current && !pending ? buildQuery(current, { ...filters, text: debounced }, roots) : null;
  const queryKey = query ? JSON.stringify(query) : null;
  const shownCount = useRef(0);
  useEffect(() => {
    if (!queryKey) return;
    let active = true;
    const q = JSON.parse(queryKey) as BrowserQuery;
    // A refresh keeps what was loaded (up to the engine's page limit).
    const limit = Math.min(MAX_LIMIT, Math.max(PAGE_SIZE, shownCount.current));
    void send(cmd("Browser", { type: "Query", query: { ...q, limit } })).then((r) => {
      if (active && r?.type === "BrowserPage") setResults({ query: queryKey, items: r.page.items, total: r.page.total });
    });
    return () => {
      active = false;
    };
  }, [send, queryKey, refresh]);
  const shown = results && results.query === queryKey ? results : null;
  useEffect(() => {
    shownCount.current = shown?.items.length ?? 0;
  });

  const loadingMore = useRef(false);
  const loadMore = () => {
    if (!shown || !queryKey || loadingMore.current || shown.items.length >= shown.total) return;
    loadingMore.current = true;
    const q = JSON.parse(queryKey) as BrowserQuery;
    void send(cmd("Browser", { type: "Query", query: { ...q, offset: shown.items.length } })).then((r) => {
      loadingMore.current = false;
      if (r?.type !== "BrowserPage") return;
      setResults((prev) =>
        prev && prev.query === queryKey && prev.items.length === r.page.offset
          ? { ...prev, items: [...prev.items, ...r.page.items], total: r.page.total }
          : prev,
      );
    });
  };

  const patchItem = (id: string, patch: Partial<LibraryItem>) =>
    setResults((prev) => prev && { ...prev, items: prev.items.map((i) => (i.id === id ? { ...i, ...patch } : i)) });

  // ---- Folder view -----------------------------------------------------------------------
  const dirPath = current && loc ? joinPath(loc.base, current.folder) : null;
  const listingKey = folderMode && current ? `${current.root}:${dirPath}` : null;
  const mediaDep = folderMode && loc?.location.type === "ProjectMedia" ? media : null;
  useEffect(() => {
    if (!listingKey || !loc || dirPath === null) return;
    let active = true;
    void send(cmd("Media", { type: "ListDirectory", location: loc.location, path: dirPath })).then((r) => {
      if (active && r?.type === "Directory") setListing({ key: listingKey, entries: r.listing.entries });
    });
    return () => {
      active = false;
    };
    // `loc` is identified by `listingKey`; `refresh` (the index changed) re-lists, e.g. a
    // folder being imported.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [send, listingKey, mediaDep, refresh]);
  // A folder import into this place is still copying (its files appear when it ends).
  const importing = useFolderImports((st) => st.jobs.some((j) => j.root === current?.root && (j.state === "reading" || j.state === "copying")));
  const projectFilter = current?.root === PROJECT_MEDIA ? text.trim().toLowerCase() : "";
  const entries = useMemo(
    () =>
      listing && listing.key === listingKey
        ? listing.entries.filter((e) => !projectFilter || e.name.toLowerCase().includes(projectFilter))
        : null,
    [listing, listingKey, projectFilter],
  );

  // ---- Navigation --------------------------------------------------------------------------
  const go = (p: Place, keepText = false) => {
    setPlace(p);
    if (!keepText) setText("");
  };
  const navigate = (folder: string) => current && go({ root: current.root, folder });

  // `file-import`: OS files dropped here are added to the project; show them there.
  const showProjectMedia = () => hasProjectMedia && go({ root: PROJECT_MEDIA, folder: "" });
  const importDrop = useImportDrop(showProjectMedia);

  const focusFirstRow = useRef(false);
  const list = useRef<HTMLUListElement>(null);
  useEffect(() => {
    if (!focusFirstRow.current || !entries) return;
    focusFirstRow.current = false;
    list.current?.querySelector<HTMLElement>(ROW_SELECTOR)?.focus();
  }, [entries]);

  const entryTarget = (entry: DirectoryEntry): PreviewTarget | null => {
    if (!loc || entry.kind !== "Audio") return null;
    const source = sourceOf(loc.location, entry, media);
    return { key: entry.path, source, item: loc.location.type === "Library" ? `${loc.location.id}/${entry.path}` : null };
  };
  const itemTarget = (item: LibraryItem): PreviewTarget | null =>
    item.kind === "Audio" && item.source ? { key: item.id, source: item.source, item: item.id } : null;

  /** Arrow keys walk the rows (previewing audio), Home/End jump; in folders → / ← open / leave. */
  const onListKeyDown = (e: KeyboardEvent<HTMLUListElement>, targetOf: (path: string) => PreviewTarget | null, entryOf?: (path: string) => DirectoryEntry | undefined) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>(ROW_SELECTOR);
    if (!row || !current) return;
    if (entryOf) {
      const entry = entryOf(row.dataset.path ?? "");
      if ((e.key === "ArrowLeft" || e.key === "Backspace") && current.folder !== "") {
        e.preventDefault();
        focusFirstRow.current = true;
        navigate(parentPath(current.folder));
        return;
      }
      if (e.key === "ArrowRight" && entry?.kind === "Directory" && loc) {
        e.preventDefault();
        focusFirstRow.current = true;
        navigate(entry.path.slice(loc.base ? loc.base.length + 1 : 0));
        return;
      }
    }
    const rows = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(ROW_SELECTOR));
    const i = rows.indexOf(row);
    const to = e.key === "ArrowDown" ? i + 1 : e.key === "ArrowUp" ? i - 1 : e.key === "Home" ? 0 : e.key === "End" ? rows.length - 1 : null;
    if (to === null) return;
    e.preventDefault();
    const next = rows[Math.max(0, Math.min(rows.length - 1, to))];
    if (!next || next === row) return;
    next.focus();
    next.scrollIntoView?.({ block: "nearest" });
    const target = targetOf(next.dataset.path ?? "");
    if (target) void preview.play(target);
  };

  // ---- Item actions ------------------------------------------------------------------------
  const setFavourite = (item: LibraryItem, favourite: boolean) =>
    void send(cmd("Browser", { type: "SetFavourite", item: item.id, favourite })).then((r) => r && patchItem(item.id, { favourite }));
  const saveTags = (item: LibraryItem, next: string[]) =>
    void send(cmd("Browser", { type: "SetTags", item: item.id, tags: next })).then((r) => r && patchItem(item.id, { tags: next }));
  const addTag = (t: string) => setTags((ts) => (ts.includes(t) ? ts : [...ts, t]));

  const setCurrentPreset = useCurrentPresets((s) => s.set);
  /** The selected track's device a preset item loads onto. */
  const presetTarget = (item: LibraryItem) => {
    const project = useProjectStore.getState().project;
    const track = useSelectionStore.getState().selectedTrack;
    const key = presetDeviceKey(item);
    if (!project || !track || !key) return null;
    return devicesOfTrack(project, track).find((d) => d.kind.type === "Builtin" && builtinKey(d.kind.device.type) === key) ?? null;
  };
  const loadPreset = (item: LibraryItem) => {
    const device = presetTarget(item);
    if (!item.preset) return;
    if (!device) {
      setOwnError(`Select a track with a ${presetDeviceKey(item)?.replace(/-/g, " ") ?? "matching"} device to load “${item.name}”`);
      return;
    }
    const preset = item.preset;
    void send(cmd("Preset", { type: "Load", device: device.id, preset })).then((r) => r && setCurrentPreset(device.id, preset, item.name));
  };
  const openProject = (item: LibraryItem) => void send(cmd("Project", { type: "Open", id: item.path }));
  const open = (item: LibraryItem) => {
    if (item.kind === "Preset") loadPreset(item);
    else if (item.kind === "Project") openProject(item);
  };

  const itemMenu = (e: MouseEvent, item: LibraryItem) => {
    const entries: ContextMenuEntry[] = [];
    if (item.kind === "Preset") {
      const device = presetTarget(item);
      entries.push({ label: device ? `Load on ${device.name}` : "Load preset", disabled: !device, onSelect: () => loadPreset(item) }, "separator");
    }
    if (item.kind === "Project") entries.push({ label: "Open project", onSelect: () => openProject(item) }, "separator");
    entries.push(
      { label: item.favourite ? "Unfavourite" : "Favourite", onSelect: () => setFavourite(item, !item.favourite) },
      { label: "Edit tags…", onSelect: () => setEditing(item) },
    );
    if (item.kind !== "Project") entries.push({ label: "Show in folder", onSelect: () => go(placeOfItem(item, current, roots)) });
    const filterBy = item.tags.filter((t) => !tags.includes(t));
    if (filterBy.length > 0) entries.push("separator", ...filterBy.map((t) => ({ label: `Filter by “${t}”`, onSelect: () => addTag(t) })));
    openContextMenu(e, entries);
  };

  // ---- Places ------------------------------------------------------------------------------
  const picker = folderPicker(transport);
  /** Desktop: index a folder in place (an engine-side path). */
  const addFolderPath = async (path: string) => {
    const before = new Set(roots.map((r) => r.id));
    const r = await send(cmd("Browser", { type: "AddFolder", path }));
    if (r?.type !== "BrowserRoots") return;
    setRoots(r.roots);
    const added = r.roots.find((x) => !before.has(x.id));
    if (added) go({ root: added.id, folder: "" });
  };
  const addFolder = async () => {
    const path = await picker?.pickFolder();
    if (path) await addFolderPath(path);
  };
  /** Web / remote engine: copy a folder of this computer into the library (`../folders`). */
  const copyFolder = (read: (signal: AbortSignal) => Promise<PickedFolder>) => {
    if (!transport) return;
    void importFolder(transport, read, {
      onRoot: (root) => {
        listRoots();
        go({ root, folder: "" });
      },
    });
  };
  const chooseFolder = async () => {
    try {
      const read = await pickFolder();
      if (read) copyFolder(read);
    } catch (e) {
      setOwnError(`Couldn’t open the folder: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  /**
   * OS drops on the browser: folders are added to the library (desktop paths: in place;
   * otherwise copied), loose audio files go to the project as before.
   */
  const importFiles = (sources: ImportSource[]) => {
    if (!transport || sources.length === 0) return;
    void importAudio(transport, sources);
    showProjectMedia();
  };
  const onPathDrop = (sources: ImportSource[]) => {
    const paths = sources.flatMap((x) => (x.kind === "path" ? [x.path] : []));
    importFiles(paths.filter(isAudioName).map((path) => ({ kind: "path" as const, path })));
    // Anything else is taken for a folder; the engine refuses what isn't one.
    for (const path of paths.filter((p) => !isAudioName(p))) void addFolderPath(path);
  };
  const dropProps = {
    onDragOver: (e: DragEvent<HTMLDivElement>) => {
      importDrop.onDragOver?.(e);
      if (hasOsFiles(e.dataTransfer) && isPathDropHost(transport)) noteHover(e, onPathDrop);
    },
    onDrop: (e: DragEvent<HTMLDivElement>) => {
      setDropOnPlaces(false);
      const items = hasOsFiles(e.dataTransfer) ? droppedItems(e.dataTransfer.items) : null;
      if (!items) return importDrop.onDrop?.(e);
      e.preventDefault();
      void items.then(({ folders, files }) => {
        for (const read of folders) copyFolder(read);
        importFiles(files.map((f) => fileSource(f)));
      });
    },
  };

  const renameFolder = (root: BrowserRoot, name: string) =>
    void send(cmd("Browser", { type: "RenameFolder", root: root.id, name })).then((r) => r?.type === "BrowserRoots" && setRoots(r.roots));
  const placeMenu = (e: MouseEvent, r: BrowserRoot) => {
    const entries: ContextMenuEntry[] = [{ label: "Rescan", onSelect: () => void send(cmd("Browser", { type: "Rescan", root: r.id })) }];
    if (r.kind === "Folder")
      entries.push(
        { label: "Rename…", onSelect: () => setRenaming(r) },
        "separator",
        {
          label: "Remove folder",
          danger: true,
          onSelect: () => void send(cmd("Browser", { type: "RemoveFolder", root: r.id })).then((x) => x && listRoots()),
        },
      );
    openContextMenu(e, entries);
  };

  const placeName = (id: string) =>
    id === ALL_ROOTS
      ? "All"
      : id === PROJECT_MEDIA
        ? (locations?.find((l) => l.location.type === "ProjectMedia")?.name ?? "Project media")
        : (() => {
            const r = roots.find((x) => x.id === id);
            return r ? rootLabel(r, locations) : id;
          })();
  const currentName = current ? placeName(current.root) : null;
  const q = text.trim();
  const folders = useMemo(
    () => (shown && current && debounced.trim() ? matchingFolders(shown.items, debounced, current, roots).filter((f) => rootLocation(f.root, roots, locations)) : []),
    [shown, current?.root, current?.folder, debounced, roots, locations], // eslint-disable-line react-hooks/exhaustive-deps
  );

  const toggleSync = () => {
    setSync(!sync);
    saveSync(!sync);
  };

  return (
    <div className="eth-browser eth-browser-v2" data-feature="browser" data-browser="v2" {...dropProps}>
      <div className="eth-browser__search">
        <Search className="eth-browser__search-icon" aria-hidden />
        <TextInput
          size="sm"
          type="search"
          className="eth-browser__search-input"
          aria-label="Search files"
          placeholder={currentName && current?.root !== ALL_ROOTS ? `Search ${currentName}` : "Search library"}
          value={text}
          disabled={!current}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape" && text) {
              e.stopPropagation();
              setText("");
            }
          }}
        />
        {text && (
          <button type="button" className="eth-browser__search-clear" aria-label="Clear search" onClick={() => setText("")}>
            <X aria-hidden />
          </button>
        )}
      </div>

      <div className="eth-browser-v2__filters">
        <div className="eth-browser-v2__kinds" role="group" aria-label="Kinds">
          {KIND_FILTERS.map((k) => (
            <Button key={k.value} size="sm" tone="ghost" active={kind === k.value} onClick={() => setKind(k.value)}>
              {k.label}
            </Button>
          ))}
        </div>
        <div className="eth-browser-v2__tools">
          <IconButton
            size="sm"
            tone="ghost"
            className={clsx("eth-browser-v2__star", favouritesOnly && "eth-browser-v2__star--on")}
            label="Favourites only"
            icon={<Star />}
            active={favouritesOnly}
            onClick={() => setFavouritesOnly(!favouritesOnly)}
          />
          <Button size="sm" tone="ghost" active={sync} title="Preview in time with the project tempo" onClick={toggleSync}>
            Sync
          </Button>
          <Select
            size="sm"
            aria-label="Sort"
            options={q ? SORTS : SORTS.filter((s) => s.value !== "Relevance")}
            value={q ? "Relevance" : sort}
            disabled={!!q}
            onChange={(v) => v !== "Relevance" && setSort(v)}
          />
        </div>
        {tags.length > 0 && (
          <div className="eth-browser-v2__tag-filters" aria-label="Tag filters" role="group">
            {tags.map((t) => (
              <Button key={t} size="sm" tone="ghost" active aria-label={`Remove tag filter ${t}`} onClick={() => setTags(tags.filter((x) => x !== t))}>
                {t}
                <X aria-hidden />
              </Button>
            ))}
          </div>
        )}
      </div>

      <div
        className={clsx("eth-browser__locations", dropOnPlaces && "eth-browser-v2__places--drop")}
        role="tablist"
        aria-label="Locations"
        onDragEnter={(e) => hasOsFiles(e.dataTransfer) && setDropOnPlaces(true)}
        onDragLeave={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && setDropOnPlaces(false)}
      >
        {(current ? [ALL_ROOTS, ...roots.map((r) => r.id), ...(hasProjectMedia ? [PROJECT_MEDIA] : [])] : []).map((id) => {
          const selected = current?.root === id;
          const root = roots.find((r) => r.id === id);
          return (
            <Button
              key={id}
              size="sm"
              tone="ghost"
              role="tab"
              aria-selected={selected}
              active={selected}
              title={root?.path ?? undefined}
              onClick={() => go({ root: id, folder: "" }, true)}
              onContextMenu={root ? (e) => placeMenu(e, root) : undefined}
            >
              {placeName(id)}
            </Button>
          );
        })}
        {transport && (
          <Button
            size="sm"
            tone="ghost"
            className="eth-browser-v2__add-folder"
            title={
              picker
                ? "Add a folder of this computer to the library (or drop one here)"
                : "Copy a folder’s audio files into the library (or drop one here)"
            }
            onClick={() => void (picker ? addFolder() : chooseFolder())}
          >
            <FolderPlus aria-hidden />
            {picker ? "Add folder…" : "Import folder…"}
          </Button>
        )}
      </div>

      {current && current.root !== ALL_ROOTS && (
        <nav className="eth-browser__crumbs" aria-label="Path">
          <button type="button" className="eth-browser__crumb" onClick={() => navigate("")}>
            {currentName}
          </button>
          {pathSegments(current.folder).map((seg) => (
            <span key={seg.path}>
              <span className="eth-browser__sep">/</span>
              <button type="button" className="eth-browser__crumb" onClick={() => navigate(seg.path)}>
                {seg.name}
              </button>
            </span>
          ))}
        </nav>
      )}

      {folderMode && current && loc ? (
        <ul
          ref={list}
          className="eth-browser__list"
          aria-label="Files"
          aria-busy={entries === null}
          onKeyDown={(e) =>
            onListKeyDown(
              e,
              (path) => {
                const entry = entries?.find((x) => x.path === path);
                return entry ? entryTarget(entry) : null;
              },
              (path) => entries?.find((x) => x.path === path),
            )
          }
        >
          {current.folder !== "" && (
            <li>
              <div
                className="eth-browser__row eth-browser__row--dir"
                role="button"
                tabIndex={0}
                data-row
                aria-label="Parent folder"
                onClick={() => navigate(parentPath(current.folder))}
                onKeyDown={(e) => e.key === "Enter" && navigate(parentPath(current.folder))}
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
              <span>{projectFilter ? "No matches" : importing ? "Importing…" : "Empty folder"}</span>
              <span className="eth-browser__empty-hint">
                {importing ? "The files show up here once they are copied." : "Audio files you add here show up to drag onto tracks."}
              </span>
            </li>
          )}
          {entries?.map((entry) => {
            const target = entryTarget(entry);
            return (
              <EntryRow
                key={entry.path}
                entry={entry}
                source={sourceOf(loc.location, entry, media)}
                previewing={preview.previewing === entry.path}
                onOpen={() => navigate(entry.path.slice(loc.base ? loc.base.length + 1 : 0))}
                onPreview={() => target && void preview.toggle(target)}
              />
            );
          })}
        </ul>
      ) : (
        <ul
          ref={list}
          className="eth-browser__list"
          aria-label={q ? "Search results" : "Results"}
          aria-busy={indexMode && !shown}
          onKeyDown={(e) =>
            onListKeyDown(e, (path) => {
              const item = shown?.items.find((x) => x.id === path);
              return item ? itemTarget(item) : null;
            })
          }
          onScroll={(e) => {
            const el = e.currentTarget;
            if (el.scrollTop + el.clientHeight >= el.scrollHeight - LOAD_MORE_MARGIN) loadMore();
          }}
        >
          {folders.map((f) => (
            <EntryRow
              key={`${f.root}/${f.path}`}
              entry={{ name: f.name, path: `${f.root}/${f.path}`, kind: "Directory", size: 0 }}
              detail={parentPath(f.path)}
              source={{ type: "Path", path: "" }}
              previewing={false}
              onOpen={() => go(placeOfFolder(f.root, f.path, current, roots))}
              onPreview={() => undefined}
            />
          ))}
          {shown && shown.items.length === 0 && folders.length === 0 && (
            <li className="eth-browser__empty">
              {favouritesOnly && !q ? <Star aria-hidden /> : <Search aria-hidden />}
              <span>{q ? "No matches" : favouritesOnly ? "No favourites" : "Nothing here"}</span>
              <span className="eth-browser__empty-hint">
                {q
                  ? `Nothing matches “${q}” here.`
                  : favouritesOnly
                    ? "Star samples and presets to find them here."
                    : "Items in this place show up here once indexed."}
              </span>
            </li>
          )}
          {shown?.items.map((item) => (
            <ItemRow
              key={item.id}
              item={item}
              previewing={preview.previewing === item.id}
              onActivate={(i) => {
                const t = itemTarget(i);
                if (t) void preview.toggle(t);
              }}
              onOpen={open}
              onFavourite={setFavourite}
              onTag={addTag}
              onContextMenu={itemMenu}
            />
          ))}
          {shown && shown.items.length < shown.total && (
            <li className="eth-browser-v2__more">
              <Button size="sm" tone="ghost" onClick={loadMore}>
                Show more
              </Button>
            </li>
          )}
        </ul>
      )}

      {(indexing || (indexMode && shown)) && (
        <div className="eth-browser-v2__status" aria-live="polite">
          {indexing ? (
            <span>
              Indexing {placeName(indexing.root)}… {indexing.scanned}
            </span>
          ) : (
            <span />
          )}
          {indexMode && shown && <span>{shown.total === 1 ? "1 item" : `${shown.total} items`}</span>}
        </div>
      )}

      {shownError && (
        <button
          type="button"
          className="eth-browser__error"
          role="alert"
          title="Dismiss"
          onClick={() => {
            setOwnError(null);
            clearError();
          }}
        >
          {shownError}
        </button>
      )}

      <FolderImportStatus copies={transport?.kind === "wasm"} />
      {transport && <BrowserImportBar transport={transport} onImport={showProjectMedia} />}
      <TagsDialog item={editing} onClose={() => setEditing(null)} onSave={saveTags} />
      <RenameFolderDialog root={renaming} onClose={() => setRenaming(null)} onSave={renameFolder} />
    </div>
  );
}

function TagsDialog({ item, onClose, onSave }: { item: LibraryItem | null; onClose(): void; onSave(item: LibraryItem, tags: string[]): void }) {
  const [value, setValue] = useState("");
  const [for_, setFor] = useState<string | null>(null);
  if (item && for_ !== item.id) {
    setFor(item.id);
    setValue(item.tags.join(", "));
  }
  if (!item && for_ !== null) setFor(null);
  const save = () => {
    if (item) onSave(item, parseTags(value));
    onClose();
  };
  return (
    <Dialog
      open={!!item}
      onClose={onClose}
      title={item ? `Tags of ${item.name}` : "Tags"}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button tone="accent" onClick={save}>
            Save
          </Button>
        </>
      }
    >
      <TextInput
        aria-label="Tags"
        placeholder="drums, punchy, dark"
        value={value}
        autoFocus
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && save()}
      />
    </Dialog>
  );
}

function RenameFolderDialog({ root, onClose, onSave }: { root: BrowserRoot | null; onClose(): void; onSave(root: BrowserRoot, name: string): void }) {
  const [value, setValue] = useState("");
  const [for_, setFor] = useState<string | null>(null);
  if (root && for_ !== root.id) {
    setFor(root.id);
    setValue(root.name);
  }
  if (!root && for_ !== null) setFor(null);
  const save = () => {
    if (root) onSave(root, value);
    onClose();
  };
  return (
    <Dialog
      open={!!root}
      onClose={onClose}
      title={root ? `Rename “${root.name}”` : "Rename folder"}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button tone="accent" onClick={save}>
            Rename
          </Button>
        </>
      }
    >
      <TextInput
        aria-label="Folder name"
        placeholder="Leave empty to use the folder’s own name"
        value={value}
        autoFocus
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && save()}
      />
    </Dialog>
  );
}
