// A plugin's web UI is one page and the files it references (scripts, styles, images),
// from its bundle or `get_resource`. The sandboxed frame (public/webclap-ui.html) loads
// nothing itself (its policy forbids it): the page collects the files here and sends them
// with the start page; the frame inlines them (scripts and styles as inline elements,
// images as `data:` URLs) and writes the result as its document. The page never parses the
// plugin's HTML as a document (nothing of it is evaluated here, not even by the app's
// policy checks): references are found textually.

/** One file of the UI (`null`: not found). */
export type ResourceLoader = (path: string) => Promise<{ mime: string; bytes: Uint8Array } | null>;

/** What the frame gets (`ether:webclap-load`). */
export interface UiPage {
  /** The start page's absolute path in the plugin's files, and its query (`?…`). */
  path: string;
  query: string;
  html: string;
  /** The referenced files, by absolute path. */
  files: Record<string, { mime: string; data: ArrayBuffer }>;
}

/** The path part of `uri` (no query or fragment), and the query. */
export function splitUri(uri: string): { path: string; query: string } {
  const m = /^([^?#]*)(\?[^#]*)?/.exec(uri);
  return { path: m?.[1] ?? uri, query: m?.[2] ?? "" };
}

/** `ref` relative to the directory of `base` (both absolute paths), or `null` (a URL). */
export function resolvePath(base: string, ref: string): string | null {
  if (/^[a-z][a-z0-9+.-]*:/i.test(ref) || ref.startsWith("//") || ref.startsWith("#")) return null;
  const { path } = splitUri(ref);
  if (!path) return null;
  const parts = path.startsWith("/") ? [] : base.split("/").slice(0, -1);
  for (const p of path.split("/")) {
    if (p === "" || p === ".") continue;
    if (p === "..") parts.pop();
    else parts.push(p);
  }
  return "/" + parts.filter(Boolean).join("/");
}

/** `src`/`href` attribute values of `html` (textually, best effort). */
export function references(html: string): string[] {
  const out = new Set<string>();
  for (const m of html.matchAll(/\b(?:src|href)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>"']+))/gi)) {
    const v = m[1] ?? m[2] ?? m[3];
    if (v) out.add(v);
  }
  return [...out];
}

/** Most files a ui page takes along, and their total size: the page is copied to its frame
 * every time the ui opens. */
export const MAX_UI_PAGE_FILES = 256;
export const MAX_UI_PAGE_BYTES = 128 * 1024 * 1024;

/** The start page `path` (with `query`) and the files it references (at most
 * `MAX_UI_PAGE_FILES`, `MAX_UI_PAGE_BYTES` in all: past that, an error). */
export async function collectUiPage(path: string, query: string, load: ResourceLoader): Promise<UiPage> {
  const page = await load(path);
  if (!page) throw new Error(`the plugin has no ui page at ${path}`);
  const html = new TextDecoder().decode(page.bytes);
  const files: UiPage["files"] = {};
  let count = 0;
  let total = page.bytes.length;
  for (const ref of references(html)) {
    const p = resolvePath(path, ref);
    if (!p || p in files) continue;
    if (++count > MAX_UI_PAGE_FILES) throw new Error(`the plugin's ui page references more than ${MAX_UI_PAGE_FILES} files`);
    const r = await load(p);
    if (!r) continue;
    total += r.bytes.length;
    if (total > MAX_UI_PAGE_BYTES) throw new Error("the plugin's ui page is too big");
    files[p] = { mime: r.mime, data: r.bytes.slice().buffer as ArrayBuffer };
  }
  return { path, query, html, files };
}

/** A `data:text/html` page (`{ path: "/", … }`, no files). */
export function dataPage(html: string): UiPage {
  return { path: "/index.html", query: "", html, files: {} };
}
