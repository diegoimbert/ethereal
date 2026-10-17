/**
 * The device library of one category (Instruments, Effects, MIDI Effects): the built-in
 * ("Factory") devices and the scanned plugins (desktop), filtered by a search and a source
 * chip (All / Factory / Plugins) or favourites (each row's star), sorted by name, most used
 * or most recent (`./libraryStats.ts`), with the scan status and "Plugin settings" in its
 * footer. It is the chain sections' add popover: a click on a row, or Enter on the active
 * one (↑ ↓ from the search field), picks it.
 *
 * Plugins (#425): a plugin installed in several formats is listed once per format, each row
 * with its format chip. The format toggles (CLAP / VST2 / VST3 / AU: the platform's formats
 * present in the plugin list, shown when there are two or more) filter the plugins, search
 * included. They are a multi-select, every format on by default and at least one on; the
 * formats turned off are remembered (`libraryStats.hiddenFormats`). Two plugins sharing a
 * format + id (VST2 uniqueID collisions) are both listed: the insert names which one.
 */
import { useContributions } from "@/ext";
import "./deviceLibrary.css";
import clsx from "clsx";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { AudioWaveform, FolderCog, Music2, Piano, Plug, Search, Star, X } from "lucide-react";
import type { DeviceCategory, DeviceDescriptor, PluginDescriptor, PluginFormat } from "@/generated";
import { openSettings } from "@/features/audio-settings/store";
import { isOffered } from "./chainUtils";
import { fetchBuiltinTypes } from "./descriptors";
import { FORMAT_LABEL, FORMAT_ORDER, platformFormats, pluginKey } from "@/features/plugins/filter";
import { useOptionalTransport, usePluginEvents, usePluginStore } from "@/features/plugins/pluginStore";
import { Badge, Button, IconButton, Select, TextInput, type SelectOption } from "@/kit";
import { cmd } from "@/transport";
import { localizeDescriptor, useT, type MessageKey } from "@/i18n";
import type { LibraryDevicePayload } from "./libraryDevice";
import { libraryKey, useLibraryStats, type LibrarySort } from "./libraryStats";
import { stepWrapped } from "./libraryNav";

export type LibrarySource = "All" | "Factory" | "Plugins";
const SORT_LABEL: Record<LibrarySort, MessageKey> = {
  Name: "devices.library.sort_name",
  MostUsed: "devices.library.sort_most_used",
  Recent: "devices.library.sort_recent",
};
const SOURCE_LABEL: Record<LibrarySource, MessageKey> = {
  All: "devices.library.source_all",
  Factory: "devices.library.source_factory",
  Plugins: "devices.library.source_plugins",
};
const SOURCES: ReadonlyArray<LibrarySource> = ["All", "Factory", "Plugins"];

/** What the list holds (an ICU `select` argument of the library's messages). */
type Noun = "instruments" | "effects" | "midi_effects" | "plugins" | "devices";
const NOUN: Record<DeviceCategory, Noun> = { Instrument: "instruments", AudioEffect: "effects", NoteEffect: "midi_effects" };
const ICON: Record<DeviceCategory, ReactNode> = { Instrument: <Piano />, AudioEffect: <AudioWaveform />, NoteEffect: <Music2 /> };

interface Entry {
  key: string;
  name: string;
  /** Plugins: vendor and format (its label, shown as the row's chip). */
  vendor?: string;
  format?: string;
  pluginFormat?: PluginFormat;
  payload: LibraryDevicePayload;
}

const builtinEntry = (d: DeviceDescriptor): Entry | null =>
  d.device_type.type === "Builtin"
    ? {
        key: `builtin:${d.device_type.device}`,
        name: d.name,
        payload: { name: d.name, category: d.category, device: { type: "Builtin", builtin: d.device_type.device } },
      }
    : null;

/** `shared`: another listed plugin has the same format + id (a VST2 uniqueID collision). */
const pluginEntry = (p: PluginDescriptor, shared: boolean): Entry => {
  const device: LibraryDevicePayload["device"] = { type: "Plugin", plugin_id: p.id, format: p.format, ...(shared && { name: p.name, shared }) };
  return {
    key: libraryKey({ device }),
    name: p.name,
    vendor: p.vendor,
    format: FORMAT_LABEL[p.format],
    pluginFormat: p.format,
    payload: { name: p.name, category: p.category, device },
  };
};

/** Each plugin once per format + id + name (the same plugin found at two paths loads the same). */
function pluginEntries(plugins: ReadonlyArray<PluginDescriptor>): Entry[] {
  const count = new Map<string, number>();
  for (const p of plugins) count.set(pluginKey(p), (count.get(pluginKey(p)) ?? 0) + 1);
  const seen = new Set<string>();
  return plugins.flatMap((p) => {
    const e = pluginEntry(p, (count.get(pluginKey(p)) ?? 0) > 1);
    if (seen.has(e.key)) return [];
    seen.add(e.key);
    return [e];
  });
}

/** Every whitespace-separated term in the name, vendor or format (case-insensitive). */
function matches(e: Entry, query: string): boolean {
  const hay = [e.name, e.vendor ?? "", e.format ?? ""].join(" ").toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((t) => hay.includes(t));
}

export interface DeviceLibraryProps {
  /** One category (Instrument / AudioEffect / NoteEffect) or several (a track's add menu). */
  category?: DeviceCategory;
  categories?: ReadonlyArray<DeviceCategory>;
  /** Narrows the list further, e.g. what fits a rack, or "not the plugin being replaced". */
  accept?(payload: LibraryDevicePayload): boolean;
  /** Plugins only (no factory devices, no source chips): Replace plugin, Resolve. */
  pluginsOnly?: boolean;
  /** A given list of installed plugins instead of the engine's scanned one (the resolver's catalog). */
  plugins?: ReadonlyArray<PluginDescriptor> | null;
  /** A click on a row, or Enter on the active one. */
  onPick(payload: LibraryDevicePayload): void;
  /** Close the surrounding popover: opening the plugin settings does. */
  onClose?(): void;
}

const ALL_CATEGORIES: ReadonlyArray<DeviceCategory> = ["Instrument", "AudioEffect", "NoteEffect"];

export function DeviceLibrary({ category, categories, accept, pluginsOnly = false, plugins: given, onPick, onClose }: DeviceLibraryProps) {
  const t = useT();
  const cats = useMemo<ReadonlyArray<DeviceCategory>>(() => categories ?? (category ? [category] : ALL_CATEGORIES), [categories, category]);
  const single = cats.length === 1 ? cats[0]! : null;
  const transport = useOptionalTransport();
  const desktop = transport?.kind === "tauri";
  usePluginEvents();
  const pluginActions = useContributions("plugins.actions");
  const stored = usePluginStore((s) => s.plugins);
  // Another computer's engine plays the project: its plugins (`usePluginStore.remote`).
  const remote = usePluginStore((s) => s.remote);
  const plugins = given !== undefined ? given : stored;
  const scan = usePluginStore((s) => s.scan);
  const failed = usePluginStore((s) => s.failed);
  const setPlugins = usePluginStore((s) => s.setPlugins);
  const [builtins, setBuiltins] = useState<DeviceDescriptor[] | null>(null);
  const [query, setQuery] = useState("");
  const [source, setSource] = useState<LibrarySource>("All");
  const [active, setActive] = useState<string | null>(null);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    if (!transport) return;
    let alive = true;
    fetchBuiltinTypes(transport).then(
      (l) => alive && setBuiltins(l),
      () => alive && setBuiltins([]),
    );
    return () => {
      alive = false;
    };
  }, [transport]);

  const listPlugins = useCallback(() => {
    if (!transport || !desktop) return;
    transport
      .send(cmd("Plugin", { type: "List" }))
      .then((r) => r.type === "Plugins" && setPlugins(r.plugins))
      .catch(() => undefined);
  }, [transport, desktop, setPlugins]);
  useEffect(listPlugins, [listPlugins]);
  // A finished scan lists the plugins again (the panel also renders without an engine).
  useEffect(() => transport?.onEvent((e) => e.type === "Plugin" && e.event.type === "ScanFinished" && listPlugins()), [transport, listPlugins]);

  const favourites = useLibraryStats((s) => s.favourites);
  const usage = useLibraryStats((s) => s.usage);
  const sort = useLibraryStats((s) => s.sort);
  const [favouritesOnly, setFavouritesOnly] = useState(false);
  const hiddenFormats = useLibraryStats((s) => s.hiddenFormats);
  const pluginsShown = desktop || given !== undefined || remote !== null;
  // The format toggles: this platform's formats present in the plugin list, when 2 or more.
  const formats = useMemo<PluginFormat[]>(() => {
    if (!pluginsShown) return [];
    const present = platformFormats().filter((f) => (plugins ?? []).some((p) => p.format === f));
    return present.length > 1 ? present : [];
  }, [plugins, pluginsShown]);
  // Formats turned off, among the toggles; never all of them (a stale setting shows everything).
  const hidden = useMemo(() => {
    const off = formats.filter((f) => hiddenFormats.has(f));
    return new Set<PluginFormat>(off.length < formats.length ? off : []);
  }, [formats, hiddenFormats]);
  const entries = useMemo(() => {
    const ok = (e: Entry) => !accept || accept(e.payload);
    const factory =
      source === "Plugins" || pluginsOnly
        ? []
        : (builtins ?? [])
            .filter((d) => isOffered(d) && cats.includes(d.category))
            .flatMap((d) => builtinEntry(localizeDescriptor(d)) ?? [])
            .filter(ok);
    const scanned =
      source === "Factory" || !pluginsShown ? [] : pluginEntries((plugins ?? []).filter((p) => cats.includes(p.category) && !hidden.has(p.format))).filter(ok);
    // A plugin's formats in toggle order (CLAP, VST2, VST3, AU).
    const rank = (e: Entry) => (e.pluginFormat ? FORMAT_ORDER.indexOf(e.pluginFormat) : -1);
    const byName = (a: Entry, b: Entry) => a.name.localeCompare(b.name) || (a.vendor ?? "").localeCompare(b.vendor ?? "") || rank(a) - rank(b);
    const used = (e: Entry) => usage[e.key];
    const order =
      sort === "MostUsed"
        ? (a: Entry, b: Entry) => (used(b)?.count ?? 0) - (used(a)?.count ?? 0) || byName(a, b)
        : sort === "Recent"
          ? (a: Entry, b: Entry) => (used(b)?.last ?? 0) - (used(a)?.last ?? 0) || byName(a, b)
          : byName;
    return [...factory, ...scanned].filter((e) => matches(e, query) && (!favouritesOnly || favourites.has(e.key))).sort(order);
  }, [builtins, plugins, pluginsShown, hidden, source, cats, accept, pluginsOnly, query, favouritesOnly, favourites, usage, sort, t]);
  // The first row is active until the arrows move it (Enter right after typing picks it).
  const activeKey = entries.some((e) => e.key === active) ? active : (entries[0]?.key ?? null);

  const onSearchKey = (ev: KeyboardEvent<HTMLInputElement>) => {
    if (ev.key === "Escape" && query) {
      ev.stopPropagation();
      setQuery("");
      return;
    }
    if (ev.key === "Enter") {
      const e = entries.find((x) => x.key === activeKey);
      if (e) onPick(e.payload);
      return;
    }
    if (ev.key !== "ArrowDown" && ev.key !== "ArrowUp") return;
    ev.preventDefault();
    const i = entries.findIndex((x) => x.key === activeKey);
    // Wraps: Up on the first row goes to the last, Down on the last to the first.
    const to = stepWrapped(i, entries.length, ev.key === "ArrowDown" ? 1 : -1);
    const next = entries[to];
    if (!next) return;
    setActive(next.key);
    Array.from(list.current?.querySelectorAll<HTMLElement>("[data-key]") ?? [])
      .find((el) => el.dataset.key === next.key)
      ?.scrollIntoView?.({ block: "nearest" });
  };

  const noun: Noun = single ? NOUN[single] : pluginsOnly ? "plugins" : "devices";
  const sortOptions: SelectOption<LibrarySort>[] = (["Name", "MostUsed", "Recent"] as const).map((value) => ({ value, label: t(SORT_LABEL[value]) }));
  return (
    <div className="eth-devlib" data-feature="device-library">
      <div className="eth-devlib__search">
        <span className="eth-devlib__field">
          <Search className="eth-devlib__search-icon" aria-hidden />
          <TextInput
            size="sm"
            type="search"
            className="eth-devlib__search-input"
            aria-label={t("devices.library.search", { kind: noun })}
            placeholder={t("devices.library.search", { kind: noun })}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onSearchKey}
            autoFocus
          />
          {query && (
            <button type="button" className="eth-devlib__search-clear" aria-label={t("devices.library.clear_search")} onClick={() => setQuery("")}>
              <X aria-hidden />
            </button>
          )}
        </span>
        <Select
          size="sm"
          className="eth-devlib__sort"
          aria-label={t("devices.library.sort")}
          options={sortOptions}
          value={sort}
          onChange={(v) => useLibraryStats.getState().setSort(v)}
        />
      </div>
      <div className="eth-devlib__sources" role="group" aria-label={t("devices.library.source")}>
        {(pluginsOnly ? [] : SOURCES).map((s) => (
          <Button key={s} size="sm" tone="ghost" active={source === s} onClick={() => setSource(s)}>
            {t(SOURCE_LABEL[s])}
          </Button>
        ))}
        {/* Inline after the sources: a bare star (yellow when on). */}
        <IconButton
          size="sm"
          tone="ghost"
          className={clsx("eth-devlib__star", favouritesOnly && "eth-devlib__star--on")}
          label={t("devices.library.favourites_only")}
          icon={<Star />}
          active={favouritesOnly}
          onClick={() => setFavouritesOnly(!favouritesOnly)}
        />
        {formats.length > 0 && source !== "Factory" && (
          // Multi-select, at least one on: the last one shown can't be turned off.
          <div className="eth-devlib__formats" role="group" aria-label={t("devices.library.formats")}>
            {formats.map((f) => {
              const on = !hidden.has(f);
              return (
                <Button
                  key={f}
                  size="sm"
                  tone="ghost"
                  active={on}
                  disabled={on && hidden.size === formats.length - 1}
                  title={on ? t("devices.library.hide_format", { format: FORMAT_LABEL[f] }) : t("devices.library.show_format", { format: FORMAT_LABEL[f] })}
                  onClick={() => useLibraryStats.getState().setFormatShown(f, !on)}
                >
                  {FORMAT_LABEL[f]}
                </Button>
              );
            })}
          </div>
        )}
      </div>
      {remote && given === undefined && (
        <p className="eth-devlib__remote" role="note">
          <Plug aria-hidden /> {t("devices.library.remote_plugins", { host: remote.name })}
        </p>
      )}
      <ul ref={list} className="eth-devlib__list" aria-label={t("devices.library.list", { kind: noun })} aria-busy={builtins === null && !pluginsOnly}>
        {entries.map((e) => (
          <li key={e.key}>
            <div
              className={clsx("eth-devlib__row", activeKey === e.key && "eth-devlib__row--active")}
              data-key={e.key}
              aria-label={e.name}
              role="button"
              title={t("devices.library.add", { name: e.name })}
              onClick={() => onPick(e.payload)}
            >
              <span className="eth-devlib__icon" aria-hidden>
                {e.format ? <Plug /> : ICON[e.payload.category]}
              </span>
              <span className={clsx("eth-devlib__name", e.format && "eth-user-text")}>{e.name}</span>
              {e.vendor && <span className="eth-devlib__vendor eth-user-text">{e.vendor}</span>}
              {e.format && <Badge className="eth-devlib__format">{e.format}</Badge>}
              <IconButton
                size="sm"
                tone="ghost"
                className={clsx("eth-devlib__star", favourites.has(e.key) && "eth-devlib__star--on")}
                label={favourites.has(e.key) ? t("devices.library.unfavourite", { name: e.name }) : t("devices.library.favourite", { name: e.name })}
                icon={<Star />}
                active={favourites.has(e.key)}
                tabIndex={-1}
                onClick={(ev) => {
                  ev.stopPropagation();
                  useLibraryStats.getState().toggleFavourite(e.key);
                }}
              />
            </div>
          </li>
        ))}
        {(builtins !== null || pluginsOnly) && entries.length === 0 && (
          <li className="eth-devlib__empty">
            {(source === "Plugins" || pluginsOnly) && !pluginsShown
              ? t("devices.library.desktop_only")
              : query
                ? t("devices.library.no_match")
                : hidden.size > 0 && (source === "Plugins" || pluginsOnly)
                  ? t("devices.library.empty_formats", { kind: noun })
                  : source === "Plugins"
                    ? t("devices.library.empty_scanned", { kind: noun })
                    : t("devices.library.empty", { kind: noun })}
          </li>
        )}
      </ul>
      <div className="eth-devlib__footer">
        <span className="eth-devlib__status" role="status">
          {remote
            ? null
            : scan
              ? scan.total > 0
                ? t("plugins.scan.progress", { done: scan.done, total: scan.total })
                : t("plugins.scan.scanning")
              : // A failure without a path is the scan itself failing (e.g. no scanner binary).
                failed.some((f) => !f.path)
                ? t("plugins.scan.scan_failed", { error: failed.find((f) => !f.path)!.message })
                : failed.length > 0
                  ? t("plugins.scan.failed_count", { count: failed.length })
                  : null}
        </span>
        {/* Extensions' plugin actions ("explore plugins"; `@/ext` "plugins.actions"). */}
        {pluginActions.map((Action, i) => (
          <Action key={i} place="library" onClose={onClose} />
        ))}
        {desktop && (
          // Plugin folders and scanning live in the settings (Rescan / Full rescan there).
          <Button
            size="sm"
            tone="ghost"
            onClick={() => {
              // The settings dialog replaces the picker (it would stay open over it).
              onClose?.();
              openSettings("plugins");
            }}
          >
            <FolderCog aria-hidden /> {t("devices.library.plugin_settings")}
          </Button>
        )}
      </div>
    </div>
  );
}
