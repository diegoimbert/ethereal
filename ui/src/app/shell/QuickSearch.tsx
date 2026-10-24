import clsx from "clsx";
import { Search } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { LibraryItem } from "@/generated";
import { SEARCH_DEBOUNCE_MS } from "@/features/browser";
import { matchesAction, useShortcutLabel } from "@/features/keymap";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { useT } from "@/i18n";
import { NO_AUTOCORRECT } from "@/kit";
import { useProjectStore } from "@/state";
import { useShellStore } from "@/state/shell";
import { cmd } from "@/transport";
import { LIBRARY_LIMIT, goTo, libraryResults, resultKindLabel, searchProject, useQuickSearch, type QuickResult } from "./quickSearchIndex";
import "./palette.css";

/** Keep in sync with the exit animation (`--select-exit-duration`); unmount fallback. */
const EXIT_MS = 140;
/** How long after opening the search takes the focus back (the palette closing behind it). */
const KEEP_FOCUS_MS = 400;

/**
 * Quick search (`quickSearchIndex.ts`): ⌘F / Ctrl+F (keymap `search.open`) anywhere, also from a
 * text field (the panels' own search fields keep their text; the web browser's find bar
 * never opens). Type, ↑/↓, Enter jumps, Escape closes. The command palette's look.
 */
export function QuickSearch() {
  const open = useQuickSearch((s) => s.open);
  const setOpen = useQuickSearch((s) => s.setOpen);
  const [mounted, setMounted] = useState(open);
  if (open && !mounted) setMounted(true);
  const closing = mounted && !open;

  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (!matchesAction("search.open", e)) return;
      e.preventDefault();
      e.stopPropagation();
      const open = !useQuickSearch.getState().open;
      if (open) useShellStore.getState().setPalette(false);
      setOpen(open);
    };
    // Capture: before a focused field or panel handles the chord.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [setOpen]);

  useEffect(() => {
    if (!closing) return;
    const t = setTimeout(() => setMounted(false), EXIT_MS);
    return () => clearTimeout(t);
  }, [closing]);

  if (!mounted) return null;
  return <SearchCard closing={closing} onClose={() => setOpen(false)} onClosed={() => setMounted(false)} />;
}

function SearchCard({ closing, onClose, onClosed }: { closing: boolean; onClose(): void; onClosed(): void }) {
  const t = useT();
  const id = useId();
  const transport = useOptionalTransport();
  const project = useProjectStore((s) => s.project);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const restoreFocus = useRef<Element | null>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [library, setLibrary] = useState<{ query: string; items: LibraryItem[] } | null>(null);
  const shortcut = useShortcutLabel("search.open");

  useEffect(() => {
    restoreFocus.current = document.activeElement;
    input.current?.focus();
    // Opened from the command palette: the palette gives the focus back to where it was as
    // it finishes closing; keep it here.
    const opened = performance.now();
    const onFocusIn = (e: FocusEvent) => {
      if (performance.now() - opened > KEEP_FOCUS_MS || !(e.target instanceof Node) || input.current?.closest(".eth-palette")?.contains(e.target)) return;
      input.current?.focus();
    };
    document.addEventListener("focusin", onFocusIn);
    return () => {
      document.removeEventListener("focusin", onFocusIn);
      if (restoreFocus.current instanceof HTMLElement) restoreFocus.current.focus({ preventScroll: true });
    };
  }, []);

  // The library's index, debounced like the browser's own search.
  const q = query.trim();
  useEffect(() => {
    if (!transport || !q) return;
    let alive = true;
    const timer = setTimeout(() => {
      transport
        .send(
          cmd("Browser", {
            type: "Query",
            query: {
              text: q,
              kinds: ["Audio", "Midi", "Preset", "Project"],
              tags: [],
              favourites_only: false,
              roots: [],
              folder: null,
              device: null,
              sort: "Relevance",
              offset: 0,
              limit: LIBRARY_LIMIT,
            },
          }),
        )
        .then(
          (r) => alive && r?.type === "BrowserPage" && setLibrary({ query: q, items: r.page.items }),
          () => undefined, // no index (folder browser, engine without a library): project results only
        );
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [transport, q]);

  // Project results first, then the library's (they arrive later: the list doesn't reshuffle).
  const shown = useMemo<QuickResult[]>(
    () => [...(project ? searchProject(project, q) : []), ...(q && library?.query === q ? libraryResults(library.items) : [])],
    [project, q, library],
  );
  const current = Math.min(active, shown.length - 1);

  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-index="${current}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [current]);

  const run = (r: QuickResult | undefined) => {
    if (!r) return;
    // The jump moves the focus (a device card) or the view: don't take it back.
    restoreFocus.current = null;
    onClose();
    goTo(r, transport);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (shown.length) setActive((current + (e.key === "ArrowDown" ? 1 : -1) + shown.length) % shown.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(shown[current]);
    }
  };

  return (
    <div
      className={clsx("eth-palette", closing && "eth-palette--closing")}
      data-testid="quick-search"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onAnimationEnd={(e) => {
        if (closing && e.target === e.currentTarget) onClosed();
      }}
    >
      <div className="eth-palette__card" role="dialog" aria-label={t("shell.search.dialog")} onKeyDown={onKeyDown}>
        <div className="eth-palette__search">
          <Search className="eth-palette__search-icon" aria-hidden />
          <input
            ref={input}
            className="eth-palette__input"
            {...NO_AUTOCORRECT}
            role="combobox"
            aria-label={t("shell.search.input")}
            aria-expanded
            aria-controls={`${id}-list`}
            aria-activedescendant={shown.length ? `${id}-${current}` : undefined}
            placeholder={t("shell.search.placeholder")}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
          />
          {shortcut && <kbd className="eth-palette__kbd">{shortcut}</kbd>}
        </div>
        <div ref={list} id={`${id}-list`} className="eth-palette__list" role="listbox" aria-label={t("shell.search.list")}>
          {shown.length === 0 && <div className="eth-palette__empty">{t(q ? "shell.search.empty" : "shell.search.hint")}</div>}
          {shown.map((r, i) => (
            // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/interactive-supports-focus -- a listbox option: the search field keeps the focus and drives it (aria-activedescendant)
            <div
              key={r.key}
              id={`${id}-${i}`}
              data-index={i}
              data-kind={r.type}
              role="option"
              aria-selected={i === current}
              className={clsx("eth-palette__item", i === current && "eth-palette__item--active")}
              onPointerMove={() => i !== current && setActive(i)}
              onClick={() => run(r)}
            >
              <span className="eth-palette__label">
                <span className="eth-user-text">{r.name}</span>
                {r.detail && (
                  <span className="eth-palette__detail">
                    {" · "}
                    <span className="eth-user-text">{r.detail}</span>
                  </span>
                )}
              </span>
              <span className="eth-palette__group">{resultKindLabel(r)}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
