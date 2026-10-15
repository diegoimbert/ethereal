/**
 * Locale-aware formatting and parsing (docs/LOCALIZATION.md "Numbers, dates, sorting").
 * Units (dB, Hz, ms, BPM, %) are never translated.
 */
import { getLocale, intlLocale } from "./runtime";

const cache = new Map<string, Intl.NumberFormat | Intl.DateTimeFormat | Intl.Collator | Intl.RelativeTimeFormat | Intl.ListFormat>();
function cached<T extends object>(kind: string, opts: object, make: (loc: string) => T): T {
  const loc = intlLocale(getLocale());
  const key = `${kind}|${loc}|${JSON.stringify(opts)}`;
  let f = cache.get(key) as T | undefined;
  if (!f) {
    f = make(loc);
    cache.set(key, f as never);
  }
  return f;
}

export function formatNumber(value: number, opts: Intl.NumberFormatOptions = {}): string {
  return cached("n", opts, (loc) => new Intl.NumberFormat(loc, opts)).format(value);
}

/**
 * A file size in the current language's units ("1.5 GB", "1,5 Go", "320 kB"): decimal
 * (1 kB = 1000 bytes) unless `binary` (1 KB = 1024 bytes, as file managers on Windows show).
 * `digits`: decimals above a megabyte (kilobytes are always whole).
 */
export function formatBytes(bytes: number, { binary = false, digits = 1 }: { binary?: boolean; digits?: number } = {}): string {
  const k = binary ? 1024 : 1000;
  const [value, unit, fraction] =
    bytes >= k ** 3 ? [bytes / k ** 3, "gigabyte", digits] : bytes >= k ** 2 ? [bytes / k ** 2, "megabyte", digits] : [Math.max(1, bytes / k), "kilobyte", 0];
  return formatNumber(value, { style: "unit", unit, unitDisplay: "short", maximumFractionDigits: fraction });
}

/**
 * `value` with exactly `digits` decimals, no grouping: what an editable number field shows
 * ("1.5" in English, "1,5" in French). Never "-0".
 */
export function formatFixed(value: number, digits = 0): string {
  const rounded = Number(value.toFixed(digits));
  return formatNumber(rounded === 0 ? 0 : rounded, { minimumFractionDigits: digits, maximumFractionDigits: digits, useGrouping: false });
}

const SPACES = /[\s\u00a0\u202f\u2009']/g;
const MINUS = /^[\u2212\u2012\u2013]/;

/**
 * A typed number, in any shipped locale: "1.5" and "1,5" both mean 1.5 everywhere (a French
 * keyboard's numpad types "."), "1 000,5" (French) and "1,000.5" (English) are grouped. A
 * lone "," followed by exactly three digits is a thousands separator in English only
 * ("1,000" → 1000 in English, 1 in French). Returns null when it isn't a number.
 */
export function parseNumber(text: string, locale: string = intlLocale(getLocale())): number | null {
  let s = text.trim().replace(SPACES, "").replace(MINUS, "-");
  if (s === "" || s === "-" || s === "+") return null;
  const lastDot = s.lastIndexOf(".");
  const lastComma = s.lastIndexOf(",");
  if (lastDot >= 0 && lastComma >= 0) {
    // Both: the last one is the decimal separator, the other groups.
    const dec = lastDot > lastComma ? "." : ",";
    const group = dec === "." ? "," : ".";
    s = s.split(group).join("").replace(dec, ".");
  } else if (lastComma >= 0) {
    const commas = s.split(",").length - 1;
    const groupsOfThree = /^[-+]?\d{1,3}(,\d{3})+$/.test(s);
    if (commas > 1 && groupsOfThree) s = s.split(",").join("");
    else if (commas === 1 && groupsOfThree && locale.startsWith("en") && !/^[-+]?0,/.test(s)) s = s.replace(",", "");
    else if (commas === 1) s = s.replace(",", ".");
    else return null;
  } else if (lastDot >= 0 && s.split(".").length > 2) {
    // "1.000.000" (German-style grouping)
    if (!/^[-+]?\d{1,3}(\.\d{3})+$/.test(s)) return null;
    s = s.split(".").join("");
  }
  if (!/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(s)) return null;
  const n = Number(s);
  return Number.isFinite(n) ? n : null;
}

export function formatDate(date: Date | number, opts: Intl.DateTimeFormatOptions = { dateStyle: "medium" }): string {
  return cached("d", opts, (loc) => new Intl.DateTimeFormat(loc, opts)).format(date);
}

export function formatDateTime(date: Date | number, opts: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" }): string {
  return cached("d", opts, (loc) => new Intl.DateTimeFormat(loc, opts)).format(date);
}

/** "3 minutes ago" / "il y a 3 minutes", picking the unit from the distance to `now`. */
export function formatRelativeTime(date: Date | number, now: Date | number = Date.now()): string {
  const diff = (+date - +now) / 1000;
  const abs = Math.abs(diff);
  const rtf = cached("r", { numeric: "auto" }, (loc) => new Intl.RelativeTimeFormat(loc, { numeric: "auto" }));
  if (abs < 45) return rtf.format(0, "second");
  if (abs < 45 * 60) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 22 * 3600) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 26 * 86400) return rtf.format(Math.round(diff / 86400), "day");
  if (abs < 320 * 86400) return rtf.format(Math.round(diff / (30 * 86400)), "month");
  return rtf.format(Math.round(diff / (365 * 86400)), "year");
}

/** "a, b and c" / "a, b et c". */
export function formatList(items: readonly string[], type: Intl.ListFormatType = "conjunction"): string {
  return cached("l", { type }, (loc) => new Intl.ListFormat(loc, { type, style: "long" })).format(items);
}

/** Sorts names the way the language does (accents, case, and "track 2" before "track 10"). */
export function compareText(a: string, b: string): number {
  return cached("c", {}, (loc) => new Intl.Collator(loc, { numeric: true, sensitivity: "base" })).compare(a, b);
}

/** Search-folded text: lowercase, accents dropped ("Échantillon" → "echantillon"). */
export function foldForSearch(s: string): string {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}
