import clsx from "clsx";
import { Search } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { DeviceDescriptor } from "@/generated";
import { fetchBuiltinTypes } from "@/features/devices/descriptors";
import { derivePaletteActions, matchesAction, paletteShortcut, setPaletteSource, useKeymapStore, useShortcutLabel } from "@/features/keymap";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { buildCommands, fuzzyScore, type PaletteCommand } from "./commands";
import { useShellStore } from "./shellStore";
import "./palette.css";

/** Keep in sync with the exit animation (`--select-exit-duration`); unmount fallback. */
const EXIT_MS = 140;
const MAX_RECENT = 6;
const RECENT_KEY = "eth.palette.recent";

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.tagName === "TEXTAREA") return true;
  return target instanceof HTMLInputElement && !["button", "checkbox", "radio", "range", "color", "file"].includes(target.type);
}

function loadRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]") as unknown;
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function saveRecent(id: string): void {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify([id, ...loadRecent().filter((x) => x !== id)].slice(0, MAX_RECENT)));
  } catch {
    /* not remembered */
  }
}

function Shortcut({ command }: { command: PaletteCommand }) {
  const label = paletteShortcut(command);
  return label ? (
    <kbd className="eth-palette__kbd" aria-hidden>
      {label}
    </kbd>
  ) : null;
}

/**
 * Command palette: ⌘K / Ctrl+K (keymap `palette.open`) anywhere (not while typing in a text field), or the rail's
 * last button. Search, ↑/↓, Enter runs, Escape closes. Recently run commands come first.
 */
export function CommandPalette() {
  const open = useShellStore((s) => s.paletteOpen);
  const setPalette = useShellStore((s) => s.setPalette);
  const [mounted, setMounted] = useState(open);
  if (open && !mounted) setMounted(true);
  const closing = mounted && !open;
  const transport = useOptionalTransport();

  // keymap: the palette's commands are actions too (listed, bindable, run by the keymap's
  // dispatcher); it lists them through this source.
  useEffect(() => {
    let devices: DeviceDescriptor[] = [];
    let alive = true;
    if (transport)
      fetchBuiltinTypes(transport).then(
        (d) => alive && (devices = d),
        () => undefined,
      );
    setPaletteSource(() => buildCommands(transport, devices));
    return () => {
      alive = false;
      setPaletteSource(null);
    };
  }, [transport]);

  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (!matchesAction("palette.open", e)) return;
      const open = useShellStore.getState().paletteOpen;
      if (!open && isTextEntry(e.target)) return;
      e.preventDefault();
      setPalette(!open);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setPalette]);

  useEffect(() => {
    if (!closing) return;
    const t = setTimeout(() => setMounted(false), EXIT_MS);
    return () => clearTimeout(t);
  }, [closing]);

  if (!mounted) return null;
  return <Palette closing={closing} onClose={() => setPalette(false)} onClosed={() => setMounted(false)} />;
}

function Palette({ closing, onClose, onClosed }: { closing: boolean; onClose(): void; onClosed(): void }) {
  const id = useId();
  const transport = useOptionalTransport();
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const restoreFocus = useRef<Element | null>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [devices, setDevices] = useState<DeviceDescriptor[]>([]);
  const [recent] = useState(loadRecent);
  // Built when the palette opens (and again once the device list arrives).
  const commands = useMemo(() => buildCommands(transport, devices), [transport, devices]);
  // Shortcut hints follow the keymap.
  useKeymapStore((s) => s.keymap);
  useKeymapStore((s) => s.paletteVersion);
  useEffect(() => void derivePaletteActions(commands), [commands]);
  const paletteKey = useShortcutLabel("palette.open");

  useEffect(() => {
    restoreFocus.current = document.activeElement;
    input.current?.focus();
    return () => {
      if (restoreFocus.current instanceof HTMLElement) restoreFocus.current.focus({ preventScroll: true });
    };
  }, []);

  useEffect(() => {
    if (!transport) return;
    let alive = true;
    fetchBuiltinTypes(transport).then(
      (d) => alive && setDevices(d),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [transport]);

  const shown = useMemo(() => {
    const q = query.trim();
    if (!q) {
      const rank = (c: PaletteCommand) => {
        const r = recent.indexOf(c.id);
        return r < 0 ? MAX_RECENT : r;
      };
      return commands.map((c, i) => ({ c, i })).sort((a, b) => rank(a.c) - rank(b.c) || a.i - b.i).map((x) => x.c);
    }
    return commands
      .map((c) => ({ c, s: Math.max(fuzzyScore(q, c.label), fuzzyScore(q, `${c.group} ${c.keywords ?? ""}`) - 400) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => b.s - a.s)
      .map((x) => x.c);
  }, [commands, query, recent]);
  const current = Math.min(active, shown.length - 1);

  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-index="${current}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [current]);

  const run = (c: PaletteCommand | undefined) => {
    if (!c) return;
    saveRecent(c.id);
    onClose();
    c.run();
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
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onAnimationEnd={(e) => {
        if (closing && e.target === e.currentTarget) onClosed();
      }}
    >
      <div className="eth-palette__card" role="dialog" aria-label="Command palette" onKeyDown={onKeyDown}>
        <div className="eth-palette__search">
          <Search className="eth-palette__search-icon" aria-hidden />
          <input
            ref={input}
            className="eth-palette__input"
            role="combobox"
            aria-label="Search commands"
            aria-expanded
            aria-controls={`${id}-list`}
            aria-activedescendant={shown.length ? `${id}-${current}` : undefined}
            placeholder="Type a command…"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
          />
          {paletteKey && <kbd className="eth-palette__kbd">{paletteKey}</kbd>}
        </div>
        <div ref={list} id={`${id}-list`} className="eth-palette__list" role="listbox" aria-label="Commands">
          {shown.length === 0 && <div className="eth-palette__empty">No matching command</div>}
          {shown.map((c, i) => (
            <div
              key={c.id}
              id={`${id}-${i}`}
              data-index={i}
              role="option"
              aria-selected={i === current}
              className={clsx("eth-palette__item", i === current && "eth-palette__item--active")}
              onPointerMove={() => i !== current && setActive(i)}
              onClick={() => run(c)}
            >
              <span className="eth-palette__label">{c.label}</span>
              <span className="eth-palette__group">{c.group}</span>
              <Shortcut command={c} />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
