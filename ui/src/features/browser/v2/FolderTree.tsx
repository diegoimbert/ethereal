/**
 * The library's folders as a tree: each browsable root is a top-level node, folders expand
 * in place (`Media::ListDirectory`, listed on first expand, re-listed when the index
 * changes) and show the files of the chosen kind. Expanded folders are remembered.
 * - click / Enter: a folder toggles, an audio file previews (click again to stop);
 * - ↑ ↓ walk the visible rows (previewing audio), Home / End jump to the ends,
 *   → expands a folder (or steps into it), ← collapses it (or steps to its parent); the
 *   same keys work from the search field (`navRef`), moving the active row without
 *   taking the focus;
 * - files drag onto tracks (the `../dragPayload.ts` payload);
 * - `reveal` expands down to a folder and focuses it (search results, "Show in folder").
 */
import clsx from "clsx";
import {
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type Ref,
  type DragEvent,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import { AudioLines, ChevronRight, Folder, FolderOpen, FolderPlus, Music, Volume2 } from "lucide-react";
import type { BrowseLocation, Command, DirectoryEntry, MediaRef, ReplyValue } from "@/generated";
import { cmd } from "@/transport";
import { useT } from "@/i18n";
import { writeBrowserDrag, type BrowserDragPayload } from "../dragPayload";
import { formatSize, sourceOf } from "../paths";
import type { PreviewTarget } from "./preview";

type Send = (command: Command) => Promise<ReplyValue | undefined>;

/** A top-level node: a library root browsable as a location. */
export interface TreeRoot {
  id: string;
  name: string;
  location: BrowseLocation;
  /** Tooltip (a user folder's path). */
  title?: string;
  /** Factory content (bundled samples): its file and folder names are app text. */
  factory?: boolean;
  /** The root's own name is user content (a user folder), not an app name ("User library"). */
  userName?: boolean;
}

/** Files shown in the tree. */
export type TreeFileKind = "Audio" | "Midi";

export interface FolderTreeProps {
  roots: ReadonlyArray<TreeRoot>;
  kind: TreeFileKind;
  send: Send;
  media: Readonly<Record<string, MediaRef>> | undefined;
  /** Bumped when the index changes: expanded folders are listed again. */
  refresh: number;
  previewing: string | null;
  onPreview(target: PreviewTarget, toggle: boolean): void;
  /** Expand down to `path` of `root` and focus it (`n` changes per request). */
  reveal: { root: string; path: string; n: number } | null;
  onRootMenu?(e: MouseEvent, root: string): void;
  /** "Add folder…" / "Import folder…" closing the tree. */
  addFolder?: { label: string; title: string; onClick(): void };
  /** OS folders dragged over the tree. */
  dropping?: boolean;
  /** Keyboard navigation driven from elsewhere (the search field). */
  navRef?: Ref<TreeNav>;
}

export interface TreeNav {
  /** Handles a navigation key (arrows, Home / End, Enter); `false` when it isn't one. */
  key(key: string): boolean;
}

const EXPANDED_KEY = "eth-browser-tree-expanded";

const keyOf = (root: string, path: string) => `${root}\u0000${path}`;

function loadExpanded(): Set<string> {
  try {
    const v = JSON.parse(localStorage.getItem(EXPANDED_KEY) ?? "[]") as unknown;
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function saveExpanded(expanded: ReadonlySet<string>): void {
  try {
    localStorage.setItem(EXPANDED_KEY, JSON.stringify([...expanded]));
  } catch {
    // Storage unavailable: the tree's state lasts for this session only.
  }
}

interface Row {
  key: string;
  root: TreeRoot;
  depth: number;
  /** `null` for a root node. */
  entry: DirectoryEntry | null;
  /** Relative folder path (folders and roots). */
  path: string;
  /** Folders and roots. */
  expanded?: boolean;
  /** An expanded folder without visible children (`"loading"` until listed). */
  placeholder?: "empty" | "loading";
}

const ROW = "[data-tree-row]";

const FILE_ICON: Record<TreeFileKind, ReactNode> = { Audio: <AudioLines />, Midi: <Music /> };

export function FolderTree({ roots, kind, send, media, refresh, previewing, onPreview, reveal, onRootMenu, addFolder, dropping, navRef }: FolderTreeProps) {
  const t = useT();
  const [expanded, setExpanded] = useState(loadExpanded);
  const [listings, setListings] = useState<ReadonlyMap<string, DirectoryEntry[]>>(new Map());
  /** The active row: focused, or driven from the search field. */
  const [active, setActive] = useState<string | null>(null);
  const list = useRef<HTMLUListElement>(null);
  const focusAfterRender = useRef<string | null>(null);

  const rowEl = (key: string) => Array.from(list.current?.querySelectorAll<HTMLElement>(ROW) ?? []).find((el) => el.dataset.key === key);

  const rootById = useMemo(() => new Map(roots.map((r) => [r.id, r])), [roots]);

  // List every expanded folder not listed yet; a refresh lists them all again.
  const listed = useRef(new Set<string>());
  const lastRefresh = useRef(refresh);
  useEffect(() => {
    if (lastRefresh.current !== refresh) {
      lastRefresh.current = refresh;
      listed.current.clear();
    }
    for (const key of expanded) {
      if (listed.current.has(key)) continue;
      const [rootId, path = ""] = key.split("\u0000");
      const root = rootById.get(rootId!);
      if (!root) continue;
      listed.current.add(key);
      void send(cmd("Media", { type: "ListDirectory", location: root.location, path })).then((r) => {
        if (r?.type === "Directory") setListings((m) => new Map(m).set(key, r.listing.entries));
        else listed.current.delete(key);
      });
    }
  }, [expanded, rootById, send, refresh]);

  const setOpen = (key: string, open: boolean) =>
    setExpanded((prev) => {
      if (prev.has(key) === open) return prev;
      const next = new Set(prev);
      if (open) next.add(key);
      else next.delete(key);
      saveExpanded(next);
      return next;
    });

  // Reveal: expand the root and every folder down to the target, then focus it (once the
  // root is listed: a folder just added shows up with the next roots).
  const revealed = useRef(0);
  useEffect(() => {
    if (!reveal || reveal.n === revealed.current || !rootById.has(reveal.root)) return;
    revealed.current = reveal.n;
    const parts = reveal.path.split("/").filter(Boolean);
    setExpanded((prev) => {
      const next = new Set(prev);
      for (let i = 0; i <= parts.length; i++) next.add(keyOf(reveal.root, parts.slice(0, i).join("/")));
      saveExpanded(next);
      return next;
    });
    focusAfterRender.current = keyOf(reveal.root, reveal.path);
  }, [reveal, rootById]);

  // The visible rows, depth first.
  const rows = useMemo(() => {
    const out: Row[] = [];
    const walk = (root: TreeRoot, path: string, depth: number) => {
      const entries = listings.get(keyOf(root.id, path));
      if (!entries) {
        // i18n-ignore: `placeholder` is the row's kind (its text is the catalog's, at render)
        out.push({ key: `${keyOf(root.id, path)}\u0000loading`, root, depth, entry: null, path, placeholder: "loading" });
        return;
      }
      const shown = entries.filter((e) => e.kind === "Directory" || e.kind === kind);
      // i18n-ignore: as above
      if (shown.length === 0) out.push({ key: `${keyOf(root.id, path)}\u0000empty`, root, depth, entry: null, path, placeholder: "empty" });
      for (const e of shown) {
        const key = keyOf(root.id, e.path);
        if (e.kind !== "Directory") {
          out.push({ key, root, depth, entry: e, path: e.path });
          continue;
        }
        const open = expanded.has(key);
        out.push({ key, root, depth, entry: e, path: e.path, expanded: open });
        if (open) walk(root, e.path, depth + 1);
      }
    };
    for (const root of roots) {
      const key = keyOf(root.id, "");
      const open = expanded.has(key);
      out.push({ key, root, depth: 0, entry: null, path: "", expanded: open });
      if (open) walk(root, "", 1);
    }
    return out;
  }, [roots, listings, expanded, kind]);

  useEffect(() => {
    const key = focusAfterRender.current;
    if (!key) return;
    const el = rowEl(key);
    if (!el) return;
    focusAfterRender.current = null;
    setActive(key);
    el.focus();
    el.scrollIntoView?.({ block: "nearest" });
  }, [rows]);

  const targetOf = (row: Row): PreviewTarget | null => {
    if (!row.entry || row.entry.kind !== "Audio") return null;
    const loc = row.root.location;
    return {
      key: row.key,
      source: sourceOf(loc, row.entry, media),
      item: loc.type === "Library" ? `${loc.id}/${row.entry.path}` : null,
    };
  };

  const activate = (row: Row) => {
    if (row.expanded !== undefined) setOpen(row.key, !row.expanded);
    else {
      const target = targetOf(row);
      if (target) onPreview(target, true);
    }
  };

  /** Moves the active row for a navigation key; `focus`: the key came from a row. */
  const navigate = (key: string, focus: boolean): boolean => {
    const nav = rows.filter((r) => !r.placeholder);
    const i = nav.findIndex((r) => r.key === active);
    const row = nav[i];
    let to: number | null = null;
    switch (key) {
      case "ArrowDown":
        to = i + 1;
        break;
      case "ArrowUp":
        to = row ? i - 1 : nav.length - 1;
        break;
      case "Home":
        to = 0;
        break;
      case "End":
        to = nav.length - 1;
        break;
      case "ArrowRight":
        if (row?.expanded === false) setOpen(row.key, true);
        else if (row?.expanded && nav[i + 1]?.depth === row.depth + 1) to = i + 1;
        break;
      case "ArrowLeft":
        if (row?.expanded) setOpen(row.key, false);
        else if (row)
          for (let j = i - 1; j >= 0; j--)
            if (nav[j]!.depth === row.depth - 1) {
              to = j;
              break;
            }
        break;
      case "Enter":
        if (row) activate(row);
        break;
      default:
        return false;
    }
    if (to === null) return true;
    const next = nav[Math.max(0, Math.min(nav.length - 1, to))];
    if (!next || next === row) return true;
    setActive(next.key);
    const el = rowEl(next.key);
    if (focus) el?.focus();
    el?.scrollIntoView?.({ block: "nearest" });
    const target = targetOf(next);
    if (target) onPreview(target, false);
    return true;
  };
  useImperativeHandle(navRef, () => ({ key: (k) => navigate(k, false) }));

  const onKeyDown = (e: KeyboardEvent<HTMLUListElement>) => {
    const el = (e.target as HTMLElement).closest<HTMLElement>(ROW);
    if (!el || (e.key === "Enter" && e.target !== el)) return;
    if (navigate(e.key, true)) e.preventDefault();
  };

  const onDragStart = (e: DragEvent<HTMLDivElement>, row: Row) => {
    if (!row.entry) return;
    const payload: BrowserDragPayload = {
      version: 1,
      kind: "media",
      source: sourceOf(row.root.location, row.entry, media),
      name: row.entry.name,
      file_kind: row.entry.kind,
    };
    writeBrowserDrag(e.dataTransfer, payload);
  };

  // The roving tab stop: the active row while shown, else the first row.
  const activeShown = rows.some((r) => r.key === active);

  return (
    <ul
      ref={list}
      className={clsx("eth-browser__list eth-browser-tree", dropping && "eth-browser-tree--drop")}
      role="tree"
      aria-label={t("browser.tree.label")}
      onKeyDown={onKeyDown}
    >
      {rows.map((row) => {
        const pad = { "--tree-depth": row.depth } as CSSProperties;
        if (row.placeholder)
          return (
            <li key={row.key} className="eth-browser-tree__placeholder" role="none" style={pad}>
              {row.placeholder === "loading" ? t("browser.tree.loading") : kind === "Audio" ? t("browser.tree.no_samples") : t("browser.tree.no_midi")}
            </li>
          );
        const isRoot = row.entry === null;
        const isFolder = row.expanded !== undefined;
        const name = row.entry?.name ?? row.root.name;
        const target = targetOf(row);
        const playing = !!target && previewing === target.key;
        const tabbable = activeShown ? active === row.key : row === rows[0];
        return (
          <li key={row.key} role="none">
            {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events -- the tree's onKeyDown handles the focused row's keys */}
            <div
              className={clsx(
                "eth-browser__row eth-browser-tree__row",
                isFolder ? "eth-browser__row--directory" : `eth-browser__row--${row.entry!.kind.toLowerCase()}`,
                isRoot && "eth-browser-tree__row--root",
                playing && "eth-browser__row--previewing",
              )}
              style={pad}
              role="treeitem"
              aria-level={row.depth + 1}
              aria-expanded={isFolder ? row.expanded : undefined}
              aria-selected={active === row.key}
              aria-label={name}
              tabIndex={tabbable ? 0 : -1}
              data-tree-row=""
              data-key={row.key}
              // Its label and title are the user's folder or file name.
              data-i18n-exempt={(isRoot ? row.root.userName : !row.root.factory) || undefined}
              title={target ? t("browser.row.preview_hint", { name }) : isRoot ? (row.root.title ?? name) : name}
              draggable={!isFolder}
              onDragStart={!isFolder ? (e) => onDragStart(e, row) : undefined}
              onFocus={() => setActive(row.key)}
              onClick={(e) => {
                e.currentTarget.focus();
                activate(row);
              }}
              onContextMenu={isRoot && onRootMenu ? (e) => onRootMenu(e, row.root.id) : undefined}
            >
              <span className={clsx("eth-browser-tree__chevron", row.expanded && "eth-browser-tree__chevron--open")} aria-hidden>
                {isFolder && <ChevronRight />}
              </span>
              <span className="eth-browser__icon" aria-hidden>
                {playing ? <Volume2 /> : isFolder ? row.expanded ? <FolderOpen /> : <Folder /> : FILE_ICON[kind]}
              </span>
              <span className={clsx("eth-browser__name", (isRoot ? row.root.userName : !row.root.factory) && "eth-user-text")}>{name}</span>
              {row.entry?.kind === "Audio" && <span className="eth-browser__size">{formatSize(row.entry.size)}</span>}
            </div>
          </li>
        );
      })}
      {addFolder && (
        <li role="none">
          <button type="button" className="eth-browser__row eth-browser-tree__add" title={addFolder.title} onClick={addFolder.onClick}>
            <span className="eth-browser-tree__chevron" aria-hidden />
            <span className="eth-browser__icon" aria-hidden>
              <FolderPlus />
            </span>
            <span className="eth-browser__name">{addFolder.label}</span>
          </button>
        </li>
      )}
    </ul>
  );
}
