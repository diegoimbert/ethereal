/**
 * Where news comes from and what the request says about this app (docs/NEWS.md, "what is sent
 * and kept"): the app version, `desktop` or `web`, and a coarse OS. No install id, no cookie.
 */
import { isTauri } from "@/transport";
import { APP_VERSION } from "@/config/version";

/**
 * The public news service run by the Ethereal maintainers (its API: docs/NEWS.md), on
 * workers.dev until the ethereal.ws zone is on the Cloudflare account. Every build, one made
 * from source included, asks it; `localStorage["eth-news-url"]` points a build at another
 * instance. Also in frame-src of scripts/release/web/_headers and
 * apps/desktop/src-tauri/tauri.conf.json: the item pages are framed from it.
 */
export const DEFAULT_NEWS_URL = "https://ethereal-news.diegoimbert.workers.dev";
/** Explicit endpoint (dev builds, e2e, self-hosting): `""` disables, a URL enables even in dev. */
export const NEWS_URL_KEY = "eth-news-url";

export interface NewsEnv {
  /** `import.meta.env.DEV` or a test run. */
  dev: boolean;
  /** `navigator.webdriver` (e2e). */
  automated: boolean;
  /** `localStorage["eth-news-url"]`, if set. */
  override: string | null;
}

export function currentNewsEnv(): NewsEnv {
  let override: string | null = null;
  try {
    override = localStorage.getItem(NEWS_URL_KEY);
  } catch {
    /* no storage */
  }
  return {
    dev: import.meta.env.DEV || import.meta.env.MODE === "test",
    automated: typeof navigator !== "undefined" && !!navigator.webdriver,
    override,
  };
}

/** The service URL, or null: dev builds and automated browsers fetch nothing unless told to. */
export function resolveNewsEndpoint(env: NewsEnv): string | null {
  if (env.override !== null) return env.override.trim().replace(/\/+$/, "") || null;
  if (env.dev || env.automated) return null;
  return DEFAULT_NEWS_URL;
}

/** A coarse OS name, from the user agent (what an item's `os` targets). */
export function detectOs(ua: string = typeof navigator === "undefined" ? "" : navigator.userAgent): string {
  if (/Android/i.test(ua)) return "android";
  if (/iPhone|iPad|iPod/i.test(ua)) return "ios";
  if (/Macintosh|Mac OS X/i.test(ua)) return "macos";
  if (/Windows/i.test(ua)) return "windows";
  if (/Linux|X11|CrOS/i.test(ua)) return "linux";
  return "other";
}

export interface NewsClientInfo {
  version: string;
  platform: "desktop" | "web";
  os: string;
}

export function clientInfo(): NewsClientInfo {
  return { version: APP_VERSION, platform: isTauri() ? "desktop" : "web", os: detectOs() };
}

/** `GET /v1/news?version=&platform=&os=`. */
export function listUrl(endpoint: string, info: NewsClientInfo): string {
  const q = new URLSearchParams({ version: info.version, platform: info.platform, os: info.os });
  return `${endpoint}/v1/news?${q.toString()}`;
}
