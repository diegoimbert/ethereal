// The virtual catalog modules' types, also for hosts that compile this file (apps/web).
// eslint-disable-next-line @typescript-eslint/triple-slash-reference
/// <reference path="./virtual.d.ts" />
/**
 * The app's languages (docs/LOCALIZATION.md). English is bundled; other catalogs are lazy
 * chunks, loaded before the first render (`initI18n`) or on a switch (`setLocale`), which
 * applies live: components that read text through `useT()` re-render.
 *
 * `t(key, args)` works anywhere (components, stores, menus built as data); in a component,
 * use `useT()` so a language switch re-renders it.
 */
import { IntlMessageFormat } from "intl-messageformat";
import type { TextKey } from "@/generated";
import en from "virtual:eth-locale/en";
import { pseudoCatalog } from "./pseudo";
import type { ArgsOf, CompiledCatalog, CompiledMessage, Locale, MessageKey, ShippedLocale } from "./types";

export interface LocaleInfo {
  code: ShippedLocale;
  /** The language's own name (an endonym): shown as is in the language picker. */
  name: string;
  load: () => Promise<CompiledCatalog>;
}

/**
 * The shipped languages. Adding one: its catalog in `locales/<code>.json`, one line here, its
 * code in `ShippedLocale` (types.ts), then `pnpm i18n check` (docs/LOCALIZATION.md).
 */
export const LOCALES: readonly LocaleInfo[] = [
  { code: "en", name: "English", load: async () => en },
  { code: "fr", name: "Français", load: async () => (await import("virtual:eth-locale/fr")).default },
];

export const DEFAULT_LOCALE: ShippedLocale = "en";
export const PSEUDO_LOCALE = "en-XA";
/** Where the chosen language is kept (on this device, like the theme; not in the project). */
export const LOCALE_STORAGE_KEY = "eth-locale";

const english: CompiledCatalog = en;
let locale: Locale = DEFAULT_LOCALE;
let catalog: CompiledCatalog = english;
let formatters = new Map<string, IntlMessageFormat>();
const listeners = new Set<() => void>();
const warned = new Set<string>();

export function isLocale(code: string | null | undefined): code is Locale {
  return code === PSEUDO_LOCALE || LOCALES.some((l) => l.code === code);
}

/** The BCP 47 tag for `Intl` (the pseudo-locale formats like English). */
export function intlLocale(l: Locale = locale): string {
  return l === PSEUDO_LOCALE ? "en" : l;
}

/** The first shipped language among the user's preferred ones (`navigator.languages`). */
export function detectLocale(preferred: readonly string[] = typeof navigator === "undefined" ? [] : navigator.languages): ShippedLocale {
  for (const tag of preferred) {
    const base = tag.toLowerCase().split(/[-_]/)[0];
    const hit = LOCALES.find((l) => l.code === base);
    if (hit) return hit.code;
  }
  return DEFAULT_LOCALE;
}

function readStored(): string | null {
  try {
    return localStorage.getItem(LOCALE_STORAGE_KEY);
  } catch {
    return null;
  }
}

/** `?locale=fr` (tests, screenshots), else the saved choice, else the system's language. */
export function initialLocale(): Locale {
  const fromUrl = typeof location === "undefined" ? null : new URLSearchParams(location.search).get("locale");
  if (isLocale(fromUrl)) return fromUrl;
  const stored = readStored();
  if (isLocale(stored)) return stored;
  return detectLocale();
}

export function getLocale(): Locale {
  return locale;
}

export function subscribeLocale(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

async function loadCatalog(l: Locale): Promise<CompiledCatalog> {
  if (l === PSEUDO_LOCALE) return pseudoCatalog(english);
  const info = LOCALES.find((x) => x.code === l);
  return info ? info.load() : english;
}

let pending = 0;
/**
 * Switches the language (live: no reload). `persist` saves it as the user's choice; the
 * startup and `?locale=` paths don't. A slower load that finishes after a newer switch is
 * dropped.
 */
export async function setLocale(next: Locale, { persist = true }: { persist?: boolean } = {}): Promise<void> {
  const ticket = ++pending;
  if (persist) {
    try {
      localStorage.setItem(LOCALE_STORAGE_KEY, next);
    } catch {
      // storage unavailable (private mode): the choice lasts this session
    }
  }
  let loaded: CompiledCatalog;
  try {
    loaded = await loadCatalog(next);
  } catch (e) {
    console.error(`i18n: could not load the "${next}" catalog`, e);
    return;
  }
  if (ticket !== pending) return;
  locale = next;
  catalog = loaded;
  formatters = new Map();
  if (typeof document !== "undefined") document.documentElement.lang = intlLocale(next);
  for (const fn of [...listeners]) fn();
}

/** Loads the startup language; hosts await it before the first render (no English flash). */
export function initI18n(): Promise<void> {
  const l = initialLocale();
  if (l === DEFAULT_LOCALE) {
    if (typeof document !== "undefined") document.documentElement.lang = DEFAULT_LOCALE;
    return Promise.resolve();
  }
  return setLocale(l, { persist: false });
}

function lookup(key: string): CompiledMessage {
  const msg = catalog[key];
  if (msg !== undefined) return msg;
  if (import.meta.env.DEV && !warned.has(`${locale}:${key}`)) {
    warned.add(`${locale}:${key}`);
    console.warn(`i18n: "${key}" is missing from the "${locale}" catalog; showing English`);
  }
  return english[key] ?? key;
}

/** Formats `key`; rich-text tags may make the result an array of parts. */
export function formatMessage(key: string, args?: Record<string, unknown>): unknown {
  const msg = lookup(key);
  if (typeof msg === "string") return msg;
  let f = formatters.get(key);
  if (!f) {
    f = new IntlMessageFormat(msg, intlLocale());
    formatters.set(key, f);
  }
  try {
    return f.format(args as Parameters<IntlMessageFormat["format"]>[0]);
  } catch (e) {
    if (import.meta.env.DEV) console.error(`i18n: could not format "${key}"`, e);
    return key;
  }
}

/** The text of `key` in the current language (English when this language lacks it). */
export function t<K extends MessageKey>(key: K, ...args: ArgsOf<K>): string {
  const out = formatMessage(key, args[0] as Record<string, unknown> | undefined);
  return Array.isArray(out) ? out.join("") : String(out);
}

/**
 * A message the engine sent (`TextKey`: undo labels…, docs/LOCALIZATION.md) in the current
 * language; `fallback` (the engine's English) when there is no key or no catalog has it.
 * Nested messages in the arguments are formatted first.
 */
export function tText(key: TextKey | null | undefined, fallback: string): string {
  if (!key || (catalog[key.id] === undefined && english[key.id] === undefined)) return fallback;
  const args: Record<string, unknown> = {};
  for (const [name, value] of Object.entries(key.args ?? {})) args[name] = typeof value === "object" && value !== null ? tText(value, "") : value;
  const out = formatMessage(key.id, args);
  return Array.isArray(out) ? out.join("") : String(out);
}

/**
 * Runs `fn` (synchronous) with English as the language, then restores the current one, so
 * every `t` inside it returns English. For search: the command palette and the shortcut
 * editor also match a command by its English name when the app is in another language.
 * Nothing re-renders and nothing is saved.
 */
export function inEnglish<T>(fn: () => T): T {
  if (catalog === english) return fn();
  const saved = { locale, catalog, formatters };
  locale = DEFAULT_LOCALE;
  catalog = english;
  formatters = englishFormatters;
  try {
    return fn();
  } finally {
    ({ locale, catalog, formatters } = saved);
  }
}
/** `inEnglish`'s formatter cache (English messages don't change, so it is kept across calls). */
const englishFormatters = new Map<string, IntlMessageFormat>();

/** For tests: back to English with a clean slate. */
export function resetI18nForTests(): void {
  locale = DEFAULT_LOCALE;
  catalog = english;
  formatters = new Map();
  warned.clear();
  pending++;
}
