/**
 * The "Resolve missing items" dialog's state (`import-resolver`; CONTRACTS.md §14.2).
 *
 * `openResolver(focus?)` asks the engine for a proposal (`Resolve::Propose` with the
 * installed plugins), starts the audio file search (`Resolve::SearchMedia`, results as
 * `Event::Resolve`) and shows the dialog, optionally focused on one device or file (the
 * right-click "Resolve…" entries). Each item has its candidates (deterministic first, an AI
 * suggestion or the user's own pick added later), the chosen one and whether to apply it.
 * `applyResolution` sends every applied choice in one `Resolve::Apply` (one undo step).
 */
import { create } from "zustand";
import type {
  DeviceCandidate,
  DeviceId,
  DeviceResolution,
  Event,
  MediaCandidate,
  MediaId,
  MediaResolution,
  MediaSource,
  MissingDeviceGroup,
  MissingMedia,
  MissingPlugin,
  PluginCandidate,
  PluginDescriptor,
  PluginResolution,
  Replacement,
} from "@/generated";
import { cmd, commandErrorText, isCommandFailed, type EngineTransport } from "@/transport";
import { t } from "@/i18n";
import { activeConnection, aiProvider, hasAiConsent, runAi, setAiConsent, wantsAi } from "./ai";
import { loadCatalog } from "./catalog";

export interface PluginItem {
  key: string;
  missing: MissingPlugin;
  candidates: PluginCandidate[];
  /** Index of the chosen candidate (`null` = leave missing). */
  choice: number | null;
  apply: boolean;
}

export interface MediaItem {
  key: string;
  missing: MissingMedia;
  candidates: MediaCandidate[];
  choice: number | null;
  apply: boolean;
  /** The search has reported this file. */
  searched: boolean;
}

/** Missing devices of one kind (placeholders for devices Ethereal could not create). */
export interface DeviceItem {
  key: string;
  missing: MissingDeviceGroup;
  candidates: DeviceCandidate[];
  /** Index of the chosen candidate (`null` = resolve later). */
  choice: number | null;
  apply: boolean;
}

/** The two kinds of missing item: plugins and devices, or audio files (samples). */
export type Section = "devices" | "samples";

export type Focus = { device: DeviceId } | { media: MediaId } | { name: string };

export interface AiState {
  status: "idle" | "consent" | "running" | "done" | "error";
  calls: number;
  message: string | null;
}

interface ResolverState {
  open: boolean;
  loading: boolean;
  /** The item the dialog is focused on (`null` = every missing item). */
  focus: string | null;
  /** Narrowed to one kind ("Resolve missing devices" / "Resolve missing samples" buttons); `null` = both. */
  section: Section | null;
  plugins: PluginItem[];
  media: MediaItem[];
  devices: DeviceItem[];
  installed: PluginDescriptor[];
  /** This host has a plugin catalog (else only audio files are listed). */
  pluginHost: boolean;
  folderSearch: boolean;
  search: { scanned: number; done: boolean } | null;
  ai: AiState;
  applying: boolean;
  error: string | null;
  /** Missing items found by the last proposal (the import report's button). */
  count: number;
  /** Mirror of [`autoShow`] for the dialog's checkbox. */
  autoShow: boolean;
}

const AI_IDLE: AiState = { status: "idle", calls: 0, message: null };

const INITIAL: ResolverState = {
  open: false,
  loading: false,
  focus: null,
  section: null,
  plugins: [],
  media: [],
  devices: [],
  installed: [],
  pluginHost: false,
  folderSearch: false,
  search: null,
  ai: AI_IDLE,
  applying: false,
  error: null,
  count: 0,
  autoShow: true,
};

export const useResolver = create<ResolverState>()(() => ({ ...INITIAL }));

/** The app's transport (set by `ImportResolverRoot`), for menu entries outside React. */
let current: EngineTransport | null = null;
let aiAbort: AbortController | null = null;

export function setResolverTransport(t: EngineTransport | null): void {
  current = t;
}

export const pluginKey = (m: { format: string; plugin_id: string }) => `p:${m.format}:${m.plugin_id}`;
export const mediaKey = (id: MediaId) => `m:${id}`;
export const deviceKey = (source: string) => `d:${source}`;

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
  return e instanceof Error ? e.message : String(e);
}

function pluginItem(m: MissingPlugin): PluginItem {
  const best = m.candidates[0];
  return { key: pluginKey(m), missing: m, candidates: m.candidates, choice: best ? 0 : null, apply: best !== undefined };
}

/** A missing device's candidates are suggestions: one is preselected, none is applied until the user says so. */
function deviceItem(m: MissingDeviceGroup): DeviceItem {
  return { key: deviceKey(m.source_id), missing: m, candidates: m.candidates, choice: m.candidates.length > 0 ? 0 : null, apply: false };
}

function focusKey(focus: Focus | undefined, plugins: PluginItem[], media: MediaItem[], devices: DeviceItem[]): string | null {
  if (!focus) return null;
  if ("device" in focus) {
    return plugins.find((p) => p.missing.devices.includes(focus.device))?.key ?? devices.find((d) => d.missing.devices.includes(focus.device))?.key ?? null;
  }
  if ("media" in focus) return media.find((m) => m.missing.media === focus.media)?.key ?? null;
  const n = focus.name.toLowerCase();
  return (
    plugins.find((p) => p.missing.name.toLowerCase() === n)?.key ??
    devices.find((d) => n.startsWith(d.missing.description.toLowerCase()))?.key ??
    media.find((m) => m.missing.name.toLowerCase() === n)?.key ??
    null
  );
}

/** Ask the engine what is missing (no dialog). Returns the count. */
export async function proposeResolution(transport: EngineTransport, focus?: Focus): Promise<number> {
  useResolver.setState({ loading: true, error: null });
  try {
    const installed = await loadCatalog(transport).catch(() => null);
    const reply = await transport.send(cmd("Resolve", { type: "Propose", installed: installed ?? [], plugins: installed !== null }));
    if (reply.type !== "ResolveProposal") throw new Error("unexpected reply");
    const { proposal } = reply;
    const plugins = proposal.plugins.map(pluginItem);
    const media: MediaItem[] = proposal.media.map((m) => ({ key: mediaKey(m.media), missing: m, candidates: [], choice: null, apply: false, searched: false }));
    const devices = (proposal.devices ?? []).map(deviceItem);
    const count = plugins.length + media.length + devices.length;
    // Refreshing the open dialog (something more turned out missing) keeps the user's choices.
    const prev = useResolver.getState();
    const keep = <T extends { key: string }>(items: T[], old: T[]): T[] => (prev.open ? items.map((it) => old.find((o) => o.key === it.key) ?? it) : items);
    useResolver.setState({
      loading: false,
      plugins: keep(plugins, prev.plugins),
      media: keep(media, prev.media),
      devices: keep(devices, prev.devices),
      installed: installed ?? [],
      pluginHost: installed !== null,
      folderSearch: proposal.folder_search,
      focus: focusKey(focus, plugins, media, devices),
      count,
      ai: AI_IDLE,
    });
    return count;
  } catch (e) {
    useResolver.setState({ loading: false, error: errorText(e) });
    return 0;
  }
}

/** Search the missing audio files (`folder`: also this engine-side folder). */
export async function searchMedia(transport: EngineTransport, folder?: string): Promise<void> {
  const ids = useResolver.getState().media.map((m) => m.missing.media);
  if (ids.length === 0) return;
  useResolver.setState({ search: { scanned: 0, done: false }, error: null });
  try {
    await transport.send(cmd("Resolve", { type: "SearchMedia", media: ids, ...(folder ? { folder } : {}) }));
  } catch (e) {
    useResolver.setState({ search: null, error: errorText(e) });
  }
}

/** Open the dialog (focused on one item when given). */
export async function openResolver(focus?: Focus, transport: EngineTransport | null = current, section: Section | null = null): Promise<void> {
  if (!transport) return;
  useResolver.setState({ open: true, focus: null, section, plugins: [], media: [], devices: [], search: null });
  const n = await proposeResolution(transport, focus);
  if (n > 0 && useResolver.getState().media.length > 0) await searchMedia(transport);
}

/**
 * On project load (owner request): list everything missing in the dialog, once, when
 * anything is (and the user hasn't turned it off). The count is kept either way (the
 * "missing" badge, the project menu).
 */
export async function openIfMissing(transport: EngineTransport | null = current): Promise<boolean> {
  if (!transport || useResolver.getState().open) return false;
  const n = await proposeResolution(transport);
  if (n === 0 || !autoShow() || useResolver.getState().open) return false;
  useResolver.setState({ open: true, focus: null, section: null, search: null });
  if (useResolver.getState().media.length > 0) await searchMedia(transport);
  return true;
}

/** How long after a project opens newly detected missing files still bring up the dialog. */
const AUTO_SHOW_WINDOW_MS = 20_000;
/** Missing files are reported one by one as the engine loads them: wait for a pause. */
const AUTO_SHOW_DEBOUNCE_MS = 250;
let armedUntil = 0;
let autoTimer: ReturnType<typeof setTimeout> | undefined;

/**
 * A project opened: show the dialog if something is missing, now or as the engine finds
 * missing audio files while it loads them (`noteMissing`), until the user closes it.
 */
export function armAutoShow(transport: EngineTransport | null = current, now = Date.now()): void {
  armedUntil = now + AUTO_SHOW_WINDOW_MS;
  scheduleAutoShow(transport);
}

/** An import is running: its report comes first, so a window armed by the previous project must not open over it. */
export function disarmAutoShow(): void {
  armedUntil = 0;
  clearTimeout(autoTimer);
}

/** The engine found a missing file (`Media::Missing`): show it (or add it to the open dialog). */
export function noteMissing(transport: EngineTransport | null = current, now = Date.now()): void {
  if (now < armedUntil) scheduleAutoShow(transport);
}

function scheduleAutoShow(transport: EngineTransport | null): void {
  clearTimeout(autoTimer);
  autoTimer = setTimeout(() => {
    if (!transport || Date.now() >= armedUntil) return;
    const s = useResolver.getState();
    if (!s.open) {
      void openIfMissing(transport);
      return;
    }
    // Open: refresh it (choices kept), searching for the files it didn't list yet.
    const before = new Set(s.media.map((m) => m.key));
    void proposeResolution(transport).then(() => {
      if (useResolver.getState().media.some((m) => !before.has(m.key))) void searchMedia(transport);
    });
  }, AUTO_SHOW_DEBOUNCE_MS);
}

const AUTO_SHOW_KEY = "eth-resolver-autoshow";

/** Show the dialog when a project with missing items opens (a per-user preference, on by default). */
export function autoShow(): boolean {
  try {
    return localStorage.getItem(AUTO_SHOW_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setAutoShow(on: boolean): void {
  try {
    if (on) localStorage.removeItem(AUTO_SHOW_KEY);
    else localStorage.setItem(AUTO_SHOW_KEY, "off");
  } catch {
    /* not persisted (storage unavailable) */
  }
  useResolver.setState({ autoShow: on });
}

export function closeResolver(): void {
  // Closed (resolved, or "Resolve later"): it doesn't come back until the next project opens.
  armedUntil = 0;
  clearTimeout(autoTimer);
  aiAbort?.abort();
  aiAbort = null;
  if (current && useResolver.getState().search && !useResolver.getState().search?.done) {
    void current.send(cmd("Resolve", { type: "CancelSearch" })).catch(() => {});
  }
  useResolver.setState({ open: false, focus: null, section: null, search: null, error: null, ai: AI_IDLE });
}

export function showAll(): void {
  useResolver.setState({ focus: null, section: null });
}

/** Mirror `Event::Resolve` (media search). */
export function applyResolveEvent(e: Event): void {
  if (e.type === "ProjectLoaded") {
    useResolver.setState({ count: 0 });
    return;
  }
  if (e.type !== "Resolve") return;
  const ev = e.event;
  switch (ev.type) {
    case "MediaProgress":
      useResolver.setState({ search: { scanned: ev.scanned, done: false } });
      break;
    case "MediaCandidates":
      useResolver.setState((s) => ({
        media: s.media.map((m) => {
          if (m.missing.media !== ev.media) return m;
          // The user's own pick (from the file dialog) stays first.
          const own = m.choice !== null && m.candidates[m.choice]?.reason === CHOSEN_FILE ? [m.candidates[m.choice]!] : [];
          const candidates = [...own, ...ev.candidates];
          const best = candidates[0];
          return { ...m, candidates, searched: true, choice: best ? 0 : null, apply: own.length > 0 ? m.apply : (best?.confidence ?? 0) >= 0.5 };
        }),
      }));
      break;
    case "MediaSearchDone":
      useResolver.setState((s) => ({ search: { scanned: ev.scanned, done: true }, media: s.media.map((m) => ({ ...m, searched: true })) }));
      break;
  }
}

// ─── Choices ──────────────────────────────────────────────────────────────────────────────

function patchPlugin(key: string, f: (p: PluginItem) => PluginItem): void {
  useResolver.setState((s) => ({ plugins: s.plugins.map((p) => (p.key === key ? f(p) : p)) }));
}

function patchMedia(key: string, f: (m: MediaItem) => MediaItem): void {
  useResolver.setState((s) => ({ media: s.media.map((m) => (m.key === key ? f(m) : m)) }));
}

function patchDevice(key: string, f: (d: DeviceItem) => DeviceItem): void {
  useResolver.setState((s) => ({ devices: s.devices.map((d) => (d.key === key ? f(d) : d)) }));
}

export function setApply(key: string, apply: boolean): void {
  if (key.startsWith("p:")) patchPlugin(key, (p) => ({ ...p, apply: apply && p.choice !== null }));
  else if (key.startsWith("d:")) patchDevice(key, (d) => ({ ...d, apply: apply && d.choice !== null }));
  else patchMedia(key, (m) => ({ ...m, apply: apply && m.choice !== null }));
}

/** Choose candidate `index` (`null` = leave missing). Choosing an AI suggestion doesn't tick it. */
export function choose(key: string, index: number | null): void {
  if (key.startsWith("p:")) {
    patchPlugin(key, (p) => ({ ...p, choice: index, apply: index !== null && p.candidates[index]?.rule !== "Ai" }));
  } else if (key.startsWith("d:")) {
    patchDevice(key, (d) => ({ ...d, choice: index, apply: index !== null }));
  } else {
    patchMedia(key, (m) => ({ ...m, choice: index, apply: index !== null }));
  }
}

/** The reasons of the user's own picks (`reason`, compared below), shown through `reasonText`. */
const CHOSEN_BY_YOU = "chosen by you";
const CHOSEN_FILE = "chosen file";

/** A candidate's reason as shown: the user's own picks in the current language; the engine's as sent. */
export function reasonText(reason: string): string {
  if (reason === CHOSEN_BY_YOU) return t("resolver.reason.chosen");
  if (reason === CHOSEN_FILE) return t("resolver.reason.chosen_file");
  return reason;
}

/** The user picked an installed plugin ("Choose another…"). */
export function chooseInstalled(key: string, p: PluginDescriptor): void {
  const candidate: PluginCandidate = {
    format: p.format,
    id: p.id,
    name: p.name,
    vendor: p.vendor,
    version: p.version,
    rule: "Manual",
    confidence: 1,
    reason: CHOSEN_BY_YOU,
    plan: { type: "Auto" },
    lost: [],
  };
  patchPlugin(key, (it) => {
    const rest = it.candidates.filter((c) => c.rule !== "Manual" && !(c.format === p.format && c.id === p.id));
    return { ...it, candidates: [candidate, ...rest], choice: 0, apply: true };
  });
}

/** The same replacement (built-in type, or plugin format + id). */
export function sameReplacement(a: Replacement, b: Replacement): boolean {
  if (a.type === "Builtin" && b.type === "Builtin") return a.device === b.device;
  if (a.type === "Plugin" && b.type === "Plugin") return a.plugin.format === b.plugin.format && a.plugin.plugin_id === b.plugin.plugin_id;
  return false;
}

/** The user picked a replacement for a missing device ("Choose another…": a built-in or an installed plugin). */
export function chooseReplacement(key: string, replacement: Replacement, name: string): void {
  const candidate: DeviceCandidate = { replacement, name, confidence: 1, reason: CHOSEN_BY_YOU };
  patchDevice(key, (d) => {
    const rest = d.candidates.filter((c) => c.reason !== CHOSEN_BY_YOU && !sameReplacement(c.replacement, replacement));
    return { ...d, candidates: [candidate, ...rest], choice: 0, apply: true };
  });
}

/** The user picked a file ("Choose file…"). */
export function chooseFile(key: string, source: MediaSource, display: string): void {
  const candidate: MediaCandidate = { source, display, confidence: 1, reason: CHOSEN_FILE };
  patchMedia(key, (m) => ({ ...m, candidates: [candidate, ...m.candidates.filter((c) => c.reason !== CHOSEN_FILE)], choice: 0, apply: true }));
}

/** Items shown (the focused one, or all). */
export function visibleKeys(s: Pick<ResolverState, "focus" | "plugins" | "media" | "devices"> & { section?: Section | null }): Set<string> {
  const devices = [...s.plugins.map((p) => p.key), ...s.devices.map((d) => d.key)];
  const samples = s.media.map((m) => m.key);
  const all = s.section === "devices" ? devices : s.section === "samples" ? samples : [...devices, ...samples];
  return new Set(s.focus && all.includes(s.focus) ? [s.focus] : all);
}

/** What `Apply` would send for the shown items. */
export function resolutions(s: Pick<ResolverState, "focus" | "plugins" | "media" | "devices"> & { section?: Section | null }): {
  plugins: PluginResolution[];
  media: MediaResolution[];
  devices: DeviceResolution[];
  keys: string[];
} {
  const shown = visibleKeys(s);
  const keys: string[] = [];
  const plugins: PluginResolution[] = [];
  for (const p of s.plugins) {
    if (!shown.has(p.key) || !p.apply || p.choice === null) continue;
    const c = p.candidates[p.choice];
    if (!c) continue;
    keys.push(p.key);
    plugins.push({
      devices: p.missing.devices,
      format: c.format,
      plugin_id: c.id,
      name: c.name,
      vendor: c.vendor,
      version: c.version,
      plan: c.plan,
    });
  }
  const media: MediaResolution[] = [];
  for (const m of s.media) {
    if (!shown.has(m.key) || !m.apply || m.choice === null) continue;
    const c = m.candidates[m.choice];
    if (!c) continue;
    keys.push(m.key);
    media.push({ media: m.missing.media, source: c.source });
  }
  const devices: DeviceResolution[] = [];
  for (const d of s.devices) {
    if (!shown.has(d.key) || !d.apply || d.choice === null) continue;
    const c = d.candidates[d.choice];
    if (!c) continue;
    keys.push(d.key);
    devices.push({ devices: d.missing.devices, replacement: c.replacement });
  }
  return { plugins, media, devices, keys };
}

/** Apply the ticked choices of the shown items (one undo step), then refresh. */
export async function applyResolution(transport: EngineTransport): Promise<boolean> {
  const s = useResolver.getState();
  const r = resolutions(s);
  if (r.plugins.length === 0 && r.media.length === 0 && r.devices.length === 0) return false;
  useResolver.setState({ applying: true, error: null });
  try {
    await transport.send(cmd("Resolve", { type: "Apply", plugins: r.plugins, media: r.media, ...(r.devices.length > 0 ? { devices: r.devices } : {}) }));
    const done = new Set(r.keys);
    useResolver.setState((st) => {
      const plugins = st.plugins.filter((p) => !done.has(p.key));
      const media = st.media.filter((m) => !done.has(m.key));
      const devices = st.devices.filter((d) => !done.has(d.key));
      const left = plugins.length + media.length + devices.length;
      return { applying: false, plugins, media, devices, count: left, open: false, focus: null };
    });
    return true;
  } catch (e) {
    useResolver.setState({ applying: false, error: errorText(e) });
    return false;
  }
}

// ─── AI ───────────────────────────────────────────────────────────────────────────────────

/** Plugin items the AI would be asked about (of the shown ones). */
export function aiItems(s: Pick<ResolverState, "focus" | "plugins" | "media" | "devices">): PluginItem[] {
  const shown = visibleKeys(s);
  return s.plugins.filter((p) => shown.has(p.key) && wantsAi(p.missing));
}

/** "Ask AI": asks for consent first (once), then runs off the UI (cancellable). */
export async function askAi(opts: { session?: Parameters<typeof runAi>[2]["session"]; consent?: boolean } = {}): Promise<void> {
  const provider = aiProvider();
  if (!provider) return;
  if (opts.consent) setAiConsent(true);
  if (!hasAiConsent()) {
    useResolver.setState({ ai: { ...useResolver.getState().ai, status: "consent" } });
    return;
  }
  const s = useResolver.getState();
  const items = aiItems(s);
  if (items.length === 0) return;
  aiAbort?.abort();
  const abort = new AbortController();
  aiAbort = abort;
  useResolver.setState({ ai: { status: "running", calls: s.ai.calls, message: null } });
  const base = s.ai.calls;
  try {
    const res = await runAi(
      items.map((i) => i.missing),
      s.installed,
      {
        provider: provider.id,
        conn: activeConnection(),
        signal: abort.signal,
        session: opts.session,
        onCall: (n) => useResolver.setState((st) => ({ ai: { ...st.ai, calls: base + n } })),
      },
      provider.label,
    );
    if (abort.signal.aborted) return;
    useResolver.setState((st) => ({
      ai: {
        status: "done",
        calls: base + res.calls,
        message:
          res.suggestions.size === 0 ? t("resolver.ai.none_found") : res.rejected.length > 0 ? t("resolver.ai.rejected", { count: res.rejected.length }) : null,
      },
      plugins: st.plugins.map((p) => {
        const c = res.suggestions.get(`${p.missing.format}:${p.missing.plugin_id}`);
        if (!c) return p;
        const rest = p.candidates.filter((x) => x.rule !== "Ai");
        // Shown first when nothing better exists, but never ticked: the user accepts it.
        const first = p.choice === null || (p.candidates[p.choice]?.confidence ?? 0) < c.confidence;
        const candidates = first ? [c, ...rest] : [...rest, c];
        return { ...p, candidates, choice: first ? 0 : p.choice, apply: first ? false : p.apply };
      }),
    }));
  } catch (e) {
    if (abort.signal.aborted) return;
    useResolver.setState((st) => ({ ai: { status: "error", calls: st.ai.calls, message: errorText(e) } }));
  } finally {
    if (aiAbort === abort) aiAbort = null;
  }
}

export function cancelAi(): void {
  aiAbort?.abort();
  aiAbort = null;
  useResolver.setState((s) => ({
    ai: { ...s.ai, status: s.ai.calls > 0 ? "done" : "idle", message: s.ai.status === "running" ? t("resolver.ai.cancelled") : s.ai.message },
  }));
}

export function declineAi(): void {
  useResolver.setState((s) => ({ ai: { ...s.ai, status: "idle" } }));
}

/** Tests. */
export function resetResolver(): void {
  armedUntil = 0;
  clearTimeout(autoTimer);
  aiAbort?.abort();
  aiAbort = null;
  useResolver.setState({ ...INITIAL, autoShow: autoShow() });
}
