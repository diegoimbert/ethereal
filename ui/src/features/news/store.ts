// news: the owner's announcements (docs/NEWS.md). After launch the app fetches the list once
// (non-blocking, with a timeout; failures are only logged), drops what the user dismissed and
// shows the newest remaining item. Dismissed ids and the "check for news" setting live in UI
// settings (localStorage `eth-news`), like the other per-device UI preferences.
import { create } from "zustand";
import { clientInfo, currentNewsEnv, listUrl, resolveNewsEndpoint, type NewsEnv } from "./config";
import { t } from "@/i18n";

export interface NewsItem {
  id: string;
  title: string;
  /** ISO 8601. */
  published_at: string;
  /** The item page (framed in a sandboxed iframe). */
  html_url: string;
}

export const SETTINGS_KEY = "eth-news";
/** Startup checks wait this long after launch (the app settles first). */
export const STARTUP_DELAY_MS = 4_000;
export const FETCH_TIMEOUT_MS = 8_000;
/** Dismissed ids kept (oldest forgotten first). */
export const DISMISSED_MAX = 500;

interface NewsSettings {
  /** Settings › Privacy › "Check for news" (default on). */
  enabled: boolean;
  dismissed: string[];
}

function readSettings(): NewsSettings {
  try {
    const raw = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "{}") as Partial<NewsSettings>;
    return {
      enabled: raw.enabled !== false,
      dismissed: Array.isArray(raw.dismissed) ? raw.dismissed.filter((x): x is string => typeof x === "string") : [],
    };
  } catch {
    return { enabled: true, dismissed: [] };
  }
}

function writeSettings(s: NewsSettings): void {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(s));
  } catch {
    /* storage full or blocked: not persisted */
  }
}

export interface NewsState extends NewsSettings {
  /** The last list fetched (null = not fetched yet). */
  items: NewsItem[] | null;
  /** Undismissed items still to show this session, newest first (`current` included). */
  queue: NewsItem[];
  /** The item in the viewer. */
  current: NewsItem | null;
  /** `current` was opened from the list ("News…"): closing it changes nothing. */
  replay: boolean;
  /** The "News…" list dialog. */
  listOpen: boolean;
  loading: boolean;
  /** Why the last fetch failed (shown in the list dialog only). */
  error: string | null;
}

const initial = (): NewsState => ({
  ...readSettings(),
  items: null,
  queue: [],
  current: null,
  replay: false,
  listOpen: false,
  loading: false,
  error: null,
});

export const useNews = create<NewsState>()(() => initial());
const set = useNews.setState;
const get = useNews.getState;

/** Tests: re-read the settings, forget the session. */
export function resetNews(): void {
  set(initial(), true);
}

function isItem(v: unknown): v is NewsItem {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.id === "string" &&
    typeof o.title === "string" &&
    typeof o.published_at === "string" &&
    typeof o.html_url === "string" &&
    /^https?:\/\//.test(o.html_url)
  );
}

export interface FetchOptions {
  fetcher?: typeof fetch;
  env?: NewsEnv;
  timeoutMs?: number;
}

/** The list for this app, or null when news is off (setting, dev build, no endpoint). */
export async function fetchNews(opts: FetchOptions = {}): Promise<NewsItem[] | null> {
  const endpoint = resolveNewsEndpoint(opts.env ?? currentNewsEnv());
  if (!get().enabled || !endpoint) return null;
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), opts.timeoutMs ?? FETCH_TIMEOUT_MS);
  try {
    const fetcher = opts.fetcher ?? ((...a: Parameters<typeof fetch>) => fetch(...a));
    const res = await fetcher(listUrl(endpoint, clientInfo()), { credentials: "omit", signal: ctrl.signal });
    if (!res.ok) throw new Error(`the news service answered ${res.status}`);
    const body = (await res.json()) as { items?: unknown };
    const items = (Array.isArray(body.items) ? body.items : []).filter(isItem);
    items.sort((a, b) => b.published_at.localeCompare(a.published_at));
    set({ items, error: null });
    return items;
  } finally {
    clearTimeout(timer);
  }
}

/** The startup check: show the newest undismissed item, if any. Never throws. */
export async function checkNews(opts: FetchOptions = {}): Promise<void> {
  try {
    const items = await fetchNews(opts);
    if (!items) return;
    const dismissed = new Set(get().dismissed);
    const queue = items.filter((i) => !dismissed.has(i.id));
    if (queue.length === 0 || get().current) return;
    set({ queue, current: queue[0]!, replay: false });
  } catch (e) {
    console.info("news: not fetched:", e instanceof Error ? e.message : e);
  }
}

function dismiss(id: string): void {
  const s = get();
  if (s.dismissed.includes(id)) return;
  const dismissed = [...s.dismissed, id].slice(-DISMISSED_MAX);
  set({ dismissed });
  writeSettings({ enabled: s.enabled, dismissed });
}

/**
 * Close the viewer. A startup item is dismissed for good; the ones not shown yet stay for the
 * next launch. An item reopened from the list just closes.
 */
export function closeNews(): void {
  const { current, replay, queue } = get();
  if (!current) return;
  if (!replay) dismiss(current.id);
  set({ current: null, replay: false, queue: replay ? queue : queue.filter((i) => i.id !== current.id) });
}

/** Dismiss the current startup item and show the next one. */
export function nextNews(): void {
  const { current, replay, queue } = get();
  if (!current || replay) return;
  dismiss(current.id);
  const rest = queue.filter((i) => i.id !== current.id);
  set({ queue: rest, current: rest[0] ?? null });
}

export function setNewsEnabled(enabled: boolean): void {
  set({ enabled });
  writeSettings({ enabled, dismissed: get().dismissed });
}

/** "News…": the list of current items (dismissed ones included), fetched again. */
export function openNewsList(opts: FetchOptions = {}): void {
  set({ listOpen: true, loading: true, error: null });
  void fetchNews(opts)
    .catch((e: unknown) => {
      set({ error: e instanceof Error && e.name !== "AbortError" ? e.message : t("news.list.unreachable") });
    })
    .finally(() => set({ loading: false }));
}

export function closeNewsList(): void {
  set({ listOpen: false });
}

/** Open an item from the list (it stays dismissed or not as it was). */
export function showNewsItem(item: NewsItem): void {
  set({ listOpen: false, current: item, replay: true });
}
