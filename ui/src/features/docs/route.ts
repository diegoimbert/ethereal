// The docs' URLs: `/docs` (the start page), `/docs/<section>` (its first page) and
// `/docs/<section>/<page>#<heading>`. Tiny and dependency-free: the web entry imports it to
// route `/docs` to its own bundle before anything else loads.

export const DOCS_BASE = "/docs";

/** The public docs, for links from outside the app. */
export const DOCS_SITE_URL = "https://ethereal.ws/docs";

/** `/docs`, `/docs/`, `/docs/…`: the docs page (standalone, no editor or engine). */
export function isDocsPage(pathname: string): boolean {
  return pathname === DOCS_BASE || pathname.startsWith(`${DOCS_BASE}/`);
}

/** A docs location: `path` is `""` (the start page), `<section>` or `<section>/<page>`. */
export interface DocsLocation {
  path: string;
  anchor: string;
}

/** Reads `/docs/<section>/<page>` (+ `#anchor`). */
export function parseDocsUrl(pathname: string, hash = ""): DocsLocation {
  const rest = isDocsPage(pathname) ? pathname.slice(DOCS_BASE.length) : "";
  const path = rest
    .split("/")
    .filter(Boolean)
    .slice(0, 2)
    .map((s) => decodeURIComponent(s).toLowerCase())
    .join("/");
  return { path, anchor: decodeURIComponent(hash.replace(/^#/, "")) };
}

/** The URL of a docs location. */
export function docsHref(path: string, anchor = ""): string {
  const base = path ? `${DOCS_BASE}/${path}` : DOCS_BASE;
  return anchor ? `${base}#${anchor}` : base;
}

/** Docs pages the app links to (the content test checks they exist). */
export const DOC_LINKS = {
  shortcuts: "reference/keyboard-shortcuts",
  /** The report form's "check the documentation". */
  help: "reference/troubleshooting",
} as const;

/** Reads a link target written in the content (`/docs/a/b#c`); null for other links. */
export function docsLinkTarget(href: string): DocsLocation | null {
  const m = /^\/docs(\/[^#?]*)?(#.*)?$/.exec(href);
  if (!m) return null;
  return parseDocsUrl(`${DOCS_BASE}${m[1] ?? ""}`, m[2] ?? "");
}
