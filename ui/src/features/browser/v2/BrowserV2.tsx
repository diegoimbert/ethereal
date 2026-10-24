/**
 * The indexed library browser (v0.2 browser v2, CONTRACTS.md §12.8), in the folder
 * browser's visual language (`../index.tsx`, `../browser.css`):
 * - kind chips (Samples, MIDI, Presets, Projects), favourites, sort and tempo-synced preview
 *   ("Sync", remembered);
 * - Samples and MIDI without a search, favourites or tags show the library's folders as a
 *   tree (`./FolderTree.tsx`: every browsable root, folders expanding in place), closed by
 *   "Add folder…" (desktop: the OS folder dialog; the folder is indexed in place) or
 *   "Import folder…" (web, remote engine: the folder's audio is copied into the engine's
 *   library, `../folders`); OS folders dropped on the browser are added the same way
 *   (desktop paths) or imported; user folders can be rescanned, renamed and removed from
 *   their context menu;
 * - otherwise: index results (`Browser::Query`, debounced, by relevance while typing) over
 *   the whole library, matching folders first (opening one reveals it in the tree);
 * - result rows preview (click, Enter, arrow keys), drag (the `../dragPayload.ts` payload),
 *   star, show tags (click: filter by it); their context menu edits favourites and tags,
 *   shows the item's folder, loads presets on the selected track's device, opens projects;
 * - paged (100 at a time, more on scroll), re-queried on `IndexChanged`; `IndexProgress`
 *   shows a status line; saved presets trigger a rescan of the User Library.
 */
import { guardLeave } from "@/features/project/leaveGuard";
import "../browser.css";
import "./browserV2.css";
import clsx from "clsx";
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent, type MouseEvent } from "react";
import { Search, Star, X } from "lucide-react";
import type { BrowseRoot, BrowserQuery, BrowserRoot, LibraryItem } from "@/generated";
import { canReveal, reveal as revealTarget, revealLabel, revealMedia } from "@/features/reveal";
import { fileSource, hasOsFiles, importAudio, isPathDropHost, noteHover, useImportDrop, type ImportSource } from "@/features/import";
import { isAudioName } from "@/features/import/sources";
import { loadPresetCommand, useCurrentPresets } from "@/features/presets";
import { sliceSampleEntries } from "@/features/drum-rack";
import { useEngineCommands, useEngineEvent } from "@/features/transport-bar/engine";
import { NO_AUTOCORRECT, Button, Dialog, DismissibleError, IconButton, Select, TextInput, openContextMenu, type ContextMenuEntry } from "@/kit";
import { devicesOfTrack, useProjectStore, useSelectionStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { Trans, useT } from "@/i18n";
import { EntryRow, ROW_SELECTOR, SEARCH_DEBOUNCE_MS, type BrowserScope } from "../index";
import { FolderTree, type TreeNav, type TreeRoot } from "./FolderTree";
import { FolderImportStatus, droppedItems, importFolder, pickFolder, type PickedFolder } from "../folders";
import { parentPath } from "../paths";
import { useBrowserReveal } from "../reveal";
import { ItemRow } from "./ItemRow";
import {
  ALL_ROOTS,
  KIND_FILTERS,
  PAGE_SIZE,
  SORTS,
  buildQuery,
  builtinKey,
  loadSync,
  matchingFolders,
  parseTags,
  presetDeviceKey,
  rootLabel,
  isFactoryRoot,
  isUserRootName,
  rootLocation,
  saveSync,
  type ChosenSort,
  type KindFilter,
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

/** The desktop transport's folder dialog (`TauriTransport.pickFolder`), duck-typed. */
const folderPicker = (t: EngineTransport | null) =>
  typeof (t as { pickFolder?: unknown } | null)?.pickFolder === "function" ? (t as unknown as { pickFolder(): Promise<string | null> }) : null;

export interface BrowserV2Props {
  /** Unused: the indexed browser shows the library only (kept for `Browser`'s signature). */
  scope?: BrowserScope;
  /** `Browser::ListRoots` (the probe's reply). */
  initialRoots: BrowserRoot[];
}

export function BrowserV2({ initialRoots }: BrowserV2Props) {
  const t = useT();
  const { transport, send, error, clearError } = useEngineCommands();
  const [ownError, setOwnError] = useState<string | null>(null);
  const media = useProjectStore((s) => s.project?.media);
  const [roots, setRoots] = useState(initialRoots);
  const [locations, setLocations] = useState<BrowseRoot[] | null>(null);
  const [text, setText] = useState("");
  const [typed, setDebounced] = useState("");
  const [kind, setKind] = useState<KindFilter>("Audio");
  const [favouritesOnly, setFavouritesOnly] = useState(false);
  const [sort, setSort] = useState<ChosenSort>("Name");
  const [tags, setTags] = useState<string[]>([]);
  const [sync, setSync] = useState(loadSync);
  const [refresh, setRefresh] = useState(0);
  const [indexing, setIndexing] = useState<{ root: string; scanned: number } | null>(null);
  const [results, setResults] = useState<Results | null>(null);
  const [activeResult, setActiveResult] = useState<{ query: string | null; key: string } | null>(null);
  const [reveal, setReveal] = useState<{ root: string; path: string; n: number } | null>(null);
  const [editing, setEditing] = useState<LibraryItem | null>(null);
  const [renaming, setRenaming] = useState<BrowserRoot | null>(null);
  /** A folder whose removal waits for its confirmation (it may delete files). */
  const [removing, setRemoving] = useState<BrowserRoot | null>(null);
  const [dropOnPlaces, setDropOnPlaces] = useState(false);
  const preview = useItemPreview(transport, send, sync, setOwnError);
  const shownError = ownError ?? error;

  const listRoots = useCallback(() => {
    void send(cmd("Browser", { type: "ListRoots" })).then((r) => {
      if (r?.type === "BrowserRoots") setRoots(r.roots);
    });
  }, [send]);

  // Browse locations: the library roots the tree shows.
  const scoped = (ls: BrowseRoot[]) => ls.filter((l) => l.location.type === "Library");
  useEffect(() => {
    let active = true;
    void send(cmd("Media", { type: "ListLocations" })).then((r) => {
      if (active) setLocations(r?.type === "Locations" ? scoped(r.locations) : []);
    });
    return () => {
      active = false;
    };
  }, [send]);

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
    if (event.type === "Preset" && event.event.type === "Changed") transport?.send(cmd("Browser", { type: "Rescan", root: "user" })).catch(() => undefined);
  });

  const filters = { text, kind, tags, favouritesOnly, sort };
  // Samples and MIDI browse the folder tree until a search, favourites or tags narrow them.
  const treeMode = (kind === "Audio" || kind === "Midi") && !text.trim() && !favouritesOnly && tags.length === 0;
  const indexMode = !treeMode;
  /** Top-level folders: the roots browsable as a location (packs show inside their root). */
  const treeRoots = useMemo(
    () =>
      roots.flatMap((r): TreeRoot[] => {
        const loc = r.kind === "Pack" ? null : rootLocation(r.id, roots, locations);
        return loc
          ? [
              {
                id: r.id,
                name: rootLabel(r, locations),
                location: loc.location,
                title: r.path ?? undefined,
                factory: isFactoryRoot(r.id),
                userName: isUserRootName(r),
              },
            ]
          : [];
      }),
    [roots, locations],
  );

  // ---- Index results ---------------------------------------------------------------------
  const debounced = text.trim() ? typed : "";
  useEffect(() => {
    if (!text.trim()) return;
    const timer = setTimeout(() => setDebounced(text), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [text]);

  // While typing, wait for the debounced text (no flash of unfiltered results).
  const pending = text.trim() !== debounced.trim();
  const query: BrowserQuery | null = indexMode && !pending ? buildQuery({ root: ALL_ROOTS, folder: "" }, { ...filters, text: debounced }, roots) : null;
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

  // ---- Navigation --------------------------------------------------------------------------
  /** Shows a folder in the tree (clears what narrows the results). */
  const showFolder = (root: string, path: string, fileKind?: KindFilter) => {
    setText("");
    setFavouritesOnly(false);
    setTags([]);
    if (fileKind === "Audio" || fileKind === "Midi") setKind(fileKind);
    else if (kind !== "Audio" && kind !== "Midi") setKind("Audio");
    setReveal((r) => ({ root, path, n: (r?.n ?? 0) + 1 }));
  };

  // Quick search (⌘F) asks to show an item: samples and MIDI files in the tree (the tree
  // expands down to the file and focuses it), presets and projects by searching their name.
  useEffect(() => {
    const take = () => {
      const r = useBrowserReveal.getState().request;
      if (!r) return;
      useBrowserReveal.setState({ request: null });
      const { item } = r;
      // `showFolder` with the file's own kind (the tree focuses the file's row).
      setFavouritesOnly(false);
      setTags([]);
      setKind(item.kind);
      if (item.kind === "Audio" || item.kind === "Midi") {
        setText("");
        setReveal((v) => ({ root: item.root, path: item.path, n: (v?.n ?? 0) + 1 }));
      } else setText(item.name);
    };
    // A request made before the browser mounted (the panel was closed), then new ones.
    const first = setTimeout(take, 0);
    const unsubscribe = useBrowserReveal.subscribe((s) => s.request && take());
    return () => {
      clearTimeout(first);
      unsubscribe();
    };
  }, []);

  // `file-import`: OS audio files dropped here are added to the project.
  const importDrop = useImportDrop();
  const list = useRef<HTMLUListElement>(null);

  const itemTarget = (item: LibraryItem): PreviewTarget | null =>
    item.kind === "Audio" && item.source ? { key: item.id, source: item.source, item: item.id } : null;

  /** Arrow keys walk the rows (previewing audio), Home/End jump. */
  const onListKeyDown = (e: KeyboardEvent<HTMLUListElement>, targetOf: (path: string) => PreviewTarget | null) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>(ROW_SELECTOR);
    if (!row) return;
    const rows = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(ROW_SELECTOR));
    const i = rows.indexOf(row);
    const to = e.key === "ArrowDown" ? i + 1 : e.key === "ArrowUp" ? i - 1 : e.key === "Home" ? 0 : e.key === "End" ? rows.length - 1 : null;
    if (to === null) return;
    e.preventDefault();
    const next = rows[Math.max(0, Math.min(rows.length - 1, to))];
    if (!next || next === row) return;
    next.focus();
    next.scrollIntoView?.({ block: "nearest" });
    setActiveResult({ query: queryKey, key: next.dataset.path ?? "" });
    const target = targetOf(next.dataset.path ?? "");
    if (target) void preview.play(target);
  };

  // ---- Item actions ------------------------------------------------------------------------
  const setFavourite = (item: LibraryItem, favourite: boolean) =>
    void send(cmd("Browser", { type: "SetFavourite", item: item.id, favourite })).then((r) => r && patchItem(item.id, { favourite }));
  const saveTags = (item: LibraryItem, next: string[]) =>
    void send(cmd("Browser", { type: "SetTags", item: item.id, tags: next })).then((r) => r && patchItem(item.id, { tags: next }));
  const addTag = (tag: string) => setTags((ts) => (ts.includes(tag) ? ts : [...ts, tag]));

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
      const kind = presetDeviceKey(item)?.replace(/-/g, " ");
      setOwnError(kind ? t("browser.preset.no_device", { device: kind, name: item.name }) : t("browser.preset.no_matching_device", { name: item.name }));
      return;
    }
    const preset = item.preset;
    void send(loadPresetCommand(device.id, preset)).then((r) => r && setCurrentPreset(device.id, preset, item.name));
  };
  // Leaving the open project: an untitled one asks to keep or discard it first.
  const openProject = (item: LibraryItem) => void guardLeave(() => send(cmd("Project", { type: "Open", id: item.path })));
  const open = (item: LibraryItem) => {
    if (item.kind === "Preset") loadPreset(item);
    else if (item.kind === "Project") openProject(item);
  };

  const itemMenu = (e: MouseEvent, item: LibraryItem) => {
    const entries: ContextMenuEntry[] = [];
    if (item.kind === "Preset") {
      const device = presetTarget(item);
      entries.push(
        {
          label: device ? t("browser.menu.load_on", { device: device.name }) : t("browser.menu.load_preset"),
          disabled: !device,
          onSelect: () => loadPreset(item),
        },
        "separator",
      );
    }
    if (item.kind === "Project") entries.push({ label: t("browser.menu.open_project"), onSelect: () => openProject(item) }, "separator");
    const slicing = sliceSampleEntries(transport, item);
    if (slicing.length > 0) entries.push(...slicing, "separator");
    entries.push(
      { label: item.favourite ? t("browser.menu.unfavourite") : t("browser.menu.favourite"), onSelect: () => setFavourite(item, !item.favourite) },
      { label: t("browser.menu.edit_tags"), onSelect: () => setEditing(item) },
    );
    if (item.kind === "Audio" || item.kind === "Midi")
      entries.push({ label: t("browser.menu.show_in_folder"), onSelect: () => showFolder(item.root, parentPath(item.path), item.kind) });
    // Desktop: the file (or the project's folder) in the OS file manager.
    if (canReveal(transport)) {
      const source = item.kind === "Audio" || item.kind === "Midi" ? item.source : null;
      if (source && source.type !== "Upload") entries.push({ label: revealLabel(), onSelect: () => void revealMedia(transport, source) });
      else if (item.kind === "Project")
        entries.push({ label: revealLabel(), onSelect: () => void revealTarget(transport, { type: "Project", id: item.path }) });
    }
    const filterBy = item.tags.filter((tag) => !tags.includes(tag));
    if (filterBy.length > 0)
      entries.push("separator", ...filterBy.map((tag) => ({ label: t("browser.menu.filter_by", { tag }), onSelect: () => addTag(tag) })));
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
    if (added) showFolder(added.id, "");
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
        showFolder(root, "");
      },
    });
  };
  const chooseFolder = async () => {
    try {
      const read = await pickFolder();
      if (read) copyFolder(read);
    } catch (e) {
      setOwnError(t("browser.folder.open_failed", { error: e instanceof Error ? e.message : String(e) }));
    }
  };

  /**
   * OS drops on the browser: folders are added to the library (desktop paths: in place;
   * otherwise copied), loose audio files go to the project as before.
   */
  const importFiles = (sources: ImportSource[]) => {
    if (!transport || sources.length === 0) return;
    void importAudio(transport, sources);
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
  const placeMenu = (e: MouseEvent, id: string) => {
    const r = roots.find((x) => x.id === id);
    if (!r) return;
    const entries: ContextMenuEntry[] = [{ label: t("browser.menu.rescan"), onSelect: () => void send(cmd("Browser", { type: "Rescan", root: r.id })) }];
    // Desktop: the place's folder in the OS file manager.
    const at = rootLocation(r.id, roots, locations);
    if (at && canReveal(transport))
      entries.push({ label: revealLabel(), onSelect: () => void revealMedia(transport, { type: "Location", location: at.location, path: at.base }) });
    if (r.kind === "Folder")
      entries.push({ label: t("browser.menu.rename"), onSelect: () => setRenaming(r) }, "separator", {
        label: t("browser.menu.remove_folder"),
        danger: true,
        onSelect: () => setRemoving(r),
      });
    openContextMenu(e, entries);
  };

  const placeName = (id: string) => {
    const r = roots.find((x) => x.id === id);
    return r ? rootLabel(r, locations) : id;
  };
  const q = text.trim();
  const folders = useMemo(
    () =>
      shown && debounced.trim()
        ? matchingFolders(shown.items, debounced, { root: ALL_ROOTS, folder: "" }, roots).filter((f) => treeRoots.some((r) => r.id === f.root))
        : [],
    [shown, debounced, roots, treeRoots],
  );

  // ---- Keyboard from the search field ---------------------------------------------------------
  // ↑ ↓ move the active row of the tree or the results (previewing audio), Enter acts on it;
  // the tree also takes ← → Home End (its search is empty). The field keeps the focus.
  const treeNav = useRef<TreeNav>(null);
  const resultKeys = [...folders.map((f) => `${f.root}/${f.path}`), ...(shown?.items.map((i) => i.id) ?? [])];
  // While searching, the first result is active until the arrows move it (Enter right after
  // typing opens it).
  const chosenKey = activeResult && activeResult.query === queryKey && resultKeys.includes(activeResult.key) ? activeResult.key : null;
  const activeKey = chosenKey ?? (queryKey !== null ? (resultKeys[0] ?? null) : null);
  const activateResult = (key: string) => {
    const f = folders.find((x) => `${x.root}/${x.path}` === key);
    if (f) return showFolder(f.root, f.path);
    const item = shown?.items.find((x) => x.id === key);
    const target = item && itemTarget(item);
    if (target) void preview.toggle(target);
    else if (item) open(item);
  };
  const moveResult = (key: string): boolean => {
    if (key !== "ArrowDown" && key !== "ArrowUp" && key !== "Enter") return false;
    const i = activeKey === null ? -1 : resultKeys.indexOf(activeKey);
    if (key === "Enter") {
      if (activeKey !== null && i >= 0) activateResult(activeKey);
      return true;
    }
    const to = key === "ArrowDown" ? Math.min(resultKeys.length - 1, i + 1) : i < 0 ? resultKeys.length - 1 : Math.max(0, i - 1);
    const next = resultKeys[to];
    if (next === undefined || next === activeKey) return true;
    setActiveResult({ query: queryKey, key: next });
    Array.from(list.current?.querySelectorAll<HTMLElement>(ROW_SELECTOR) ?? [])
      .find((el) => el.dataset.path === next)
      ?.scrollIntoView?.({ block: "nearest" });
    const item = shown?.items.find((x) => x.id === next);
    const target = item && itemTarget(item);
    if (target) void preview.play(target);
    if (to === resultKeys.length - 1) loadMore();
    return true;
  };
  const onSearchKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape" && text) {
      e.stopPropagation();
      setText("");
      return;
    }
    if (e.altKey || e.metaKey || e.ctrlKey || e.shiftKey) return;
    if (treeMode ? treeNav.current?.key(e.key) : moveResult(e.key)) e.preventDefault();
  };

  const toggleSync = () => {
    setSync(!sync);
    saveSync(!sync);
  };

  return (
    <div className="eth-browser eth-browser-v2" data-feature="browser" data-browser="v2" {...dropProps}>
      <div className="eth-browser__search">
        <span className="eth-browser-v2__field">
          <Search className="eth-browser__search-icon" aria-hidden />
          <TextInput
            size="sm"
            type="search"
            {...NO_AUTOCORRECT}
            className="eth-browser__search-input"
            aria-label={t("browser.search.label")}
            placeholder={t(KIND_FILTERS.find((k) => k.value === kind)?.search ?? "browser.search.library")}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={onSearchKeyDown}
          />
          {text && (
            <button type="button" className="eth-browser__search-clear" aria-label={t("browser.search.clear")} onClick={() => setText("")}>
              <X aria-hidden />
            </button>
          )}
        </span>
        {/* Sort, right of the search field (results only: the tree is by name). */}
        <Select
          size="sm"
          className="eth-browser-v2__sort"
          aria-label={t("browser.sort.label")}
          options={(q ? SORTS : SORTS.filter((s) => s.value !== "Relevance")).map((s) => ({ value: s.value, label: t(s.label) }))}
          value={q ? "Relevance" : sort}
          disabled={!!q || treeMode}
          onChange={(v) => v !== "Relevance" && setSort(v)}
        />
      </div>

      <div className="eth-browser-v2__filters">
        <div className="eth-browser-v2__kinds" role="group" aria-label={t("browser.kind.label")}>
          {KIND_FILTERS.map((k) => (
            <Button key={k.value} size="sm" tone="ghost" active={kind === k.value} onClick={() => setKind(k.value)}>
              {t(k.label)}
            </Button>
          ))}
          {/* Inline after the kinds: a bare star (yellow when on, no button background). */}
          <IconButton
            size="sm"
            tone="ghost"
            className={clsx("eth-browser-v2__star", favouritesOnly && "eth-browser-v2__star--on")}
            label={t("browser.favourites_only")}
            icon={<Star />}
            active={favouritesOnly}
            onClick={() => setFavouritesOnly(!favouritesOnly)}
          />
        </div>
        {tags.length > 0 && (
          <div className="eth-browser-v2__tag-filters" aria-label={t("browser.tag_filters")} role="group">
            {tags.map((tag) => (
              <Button
                key={tag}
                size="sm"
                tone="ghost"
                active
                aria-label={t("browser.remove_tag_filter", { tag })}
                onClick={() => setTags(tags.filter((x) => x !== tag))}
              >
                <span className="eth-user-text">{tag}</span>
                <X aria-hidden />
              </Button>
            ))}
          </div>
        )}
      </div>

      {treeMode ? (
        <div
          className="eth-browser-v2__tree"
          onDragEnter={(e) => hasOsFiles(e.dataTransfer) && setDropOnPlaces(true)}
          onDragLeave={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && setDropOnPlaces(false)}
        >
          <FolderTree
            roots={treeRoots}
            kind={kind}
            send={send}
            media={media}
            refresh={refresh}
            previewing={preview.previewing}
            onPreview={(target, toggle) => void (toggle ? preview.toggle(target) : preview.play(target))}
            reveal={reveal}
            navRef={treeNav}
            onRootMenu={placeMenu}
            dropping={dropOnPlaces}
            addFolder={
              transport
                ? {
                    label: picker ? t("browser.folder.add") : t("browser.folder.import"),
                    title: picker ? t("browser.folder.add_hint") : t("browser.folder.import_hint"),
                    onClick: () => void (picker ? addFolder() : chooseFolder()),
                  }
                : undefined
            }
          />
        </div>
      ) : (
        <ul
          ref={list}
          className="eth-browser__list"
          aria-label={q ? t("browser.results.search") : t("browser.results.label")}
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
          // A clicked row becomes the active one (the search field's arrows go on from it).
          onClickCapture={(e) => {
            const row = (e.target as HTMLElement).closest<HTMLElement>(ROW_SELECTOR);
            if (row?.dataset.path) setActiveResult({ query: queryKey, key: row.dataset.path });
          }}
        >
          {folders.map((f) => (
            <EntryRow
              key={`${f.root}/${f.path}`}
              entry={{ name: f.name, path: `${f.root}/${f.path}`, kind: "Directory", size: 0 }}
              detail={parentPath(f.path)}
              userText={!isFactoryRoot(f.root)}
              source={{ type: "Path", path: "" }}
              previewing={false}
              active={activeKey === `${f.root}/${f.path}`}
              onOpen={() => showFolder(f.root, f.path)}
              onPreview={() => undefined}
            />
          ))}
          {shown && shown.items.length === 0 && folders.length === 0 && (
            <li className="eth-browser__empty">
              {favouritesOnly && !q ? <Star aria-hidden /> : <Search aria-hidden />}
              <span>{q ? t("browser.empty.no_matches") : favouritesOnly ? t("browser.empty.no_favourites") : t("browser.empty.nothing_here")}</span>
              <span className="eth-browser__empty-hint">
                {q ? t("browser.empty.nothing_matches", { query: q }) : favouritesOnly ? t("browser.empty.favourites_hint") : t("browser.empty.indexed_hint")}
              </span>
            </li>
          )}
          {shown?.items.map((item) => (
            <ItemRow
              key={item.id}
              item={item}
              previewing={preview.previewing === item.id}
              active={activeKey === item.id}
              onActivate={(i) => {
                const target = itemTarget(i);
                if (target) void preview.toggle(target);
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
                {t("browser.show_more")}
              </Button>
            </li>
          )}
        </ul>
      )}

      {/* Footer: tempo-synced preview, indexing progress, the result count. */}
      <div className="eth-browser-v2__status">
        <Button size="sm" tone="ghost" active={sync} title={t("browser.sync_hint")} onClick={toggleSync}>
          {t("browser.sync")}
        </Button>
        <span className="eth-browser-v2__status-text" aria-live="polite">
          {indexing
            ? t("browser.indexing", { place: placeName(indexing.root), count: indexing.scanned })
            : indexMode && shown
              ? t("browser.items", { count: shown.total })
              : ""}
        </span>
      </div>

      {shownError && (
        <DismissibleError
          className="eth-browser__error"
          onDismiss={() => {
            setOwnError(null);
            clearError();
          }}
        >
          {shownError}
        </DismissibleError>
      )}

      <FolderImportStatus copies={transport?.kind === "wasm"} />
      <TagsDialog item={editing} onClose={() => setEditing(null)} onSave={saveTags} />
      <RenameFolderDialog root={renaming} onClose={() => setRenaming(null)} onSave={renameFolder} />
      <RemoveFolderDialog
        root={removing}
        copies={transport?.kind === "wasm"}
        onClose={() => setRemoving(null)}
        onRemove={(r) => void send(cmd("Browser", { type: "RemoveFolder", root: r.id })).then((x) => x && listRoots())}
      />
    </div>
  );
}

function TagsDialog({ item, onClose, onSave }: { item: LibraryItem | null; onClose(): void; onSave(item: LibraryItem, tags: string[]): void }) {
  const t = useT();
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
      title={
        item ? (
          <Trans k="browser.tags.title_of" values={{ name: <span className={isFactoryRoot(item.root) ? undefined : "eth-user-text"}>{item.name}</span> }} />
        ) : (
          t("browser.tags.title")
        )
      }
      footer={
        <>
          <Button onClick={onClose}>{t("browser.dialog.cancel")}</Button>
          <Button tone="accent" onClick={save}>
            {t("browser.dialog.save")}
          </Button>
        </>
      }
    >
      <TextInput
        {...NO_AUTOCORRECT}
        aria-label={t("browser.tags.title")}
        placeholder={t("browser.tags.placeholder")}
        value={value}
        autoFocus
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && save()}
      />
    </Dialog>
  );
}

/**
 * "remove folder…" asks first, like every other delete that can't be undone: an
 * imported folder's copy of the files is deleted (always on the web), and clips using
 * them lose their audio; a folder used in place is only forgotten.
 */
function RemoveFolderDialog({
  root,
  copies,
  onClose,
  onRemove,
}: {
  root: BrowserRoot | null;
  /** Folders are always imported copies here (the web build). */
  copies: boolean;
  onClose(): void;
  onRemove(root: BrowserRoot): void;
}) {
  const t = useT();
  return (
    <Dialog
      open={!!root}
      onClose={onClose}
      title={root ? <Trans k="browser.remove.title_of" values={{ name: <span className="eth-user-text">{root.name}</span> }} /> : t("browser.remove.title")}
      footer={
        <>
          <Button onClick={onClose}>{t("browser.dialog.cancel")}</Button>
          <Button
            tone="danger"
            onClick={() => {
              if (root) onRemove(root);
              onClose();
            }}
          >
            {t("browser.remove.confirm")}
          </Button>
        </>
      }
    >
      <p data-testid="remove-folder-warning">{copies ? t("browser.remove.warning_copy") : t("browser.remove.warning")}</p>
    </Dialog>
  );
}

function RenameFolderDialog({ root, onClose, onSave }: { root: BrowserRoot | null; onClose(): void; onSave(root: BrowserRoot, name: string): void }) {
  const t = useT();
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
      title={root ? <Trans k="browser.rename.title_of" values={{ name: <span className="eth-user-text">{root.name}</span> }} /> : t("browser.rename.title")}
      footer={
        <>
          <Button onClick={onClose}>{t("browser.dialog.cancel")}</Button>
          <Button tone="accent" onClick={save}>
            {t("browser.rename.confirm")}
          </Button>
        </>
      }
    >
      <TextInput
        aria-label={t("browser.rename.name")}
        placeholder={t("browser.rename.placeholder")}
        value={value}
        autoFocus
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && save()}
      />
    </Dialog>
  );
}
