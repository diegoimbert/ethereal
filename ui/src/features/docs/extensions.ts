// Docs pages a build adds to the bundled ones (docs-site/content): a product built on this repo
// documents its own features with pages in the same format (docs-site/README.md), in sections
// of their own or in the existing ones. No Markdown here, so importing this module doesn't pull
// the docs into a chunk.
import type { Locale } from "@/i18n";

export interface DocsExtension {
  /** `{ "<section>/<page>.md": source }`, like docs-site/content. A page with a bundled page's path replaces it. */
  pages: Readonly<Record<string, string>>;
  /** Translated pages per language, like docs-site/content-<lang>. */
  translations?: Partial<Record<Locale, Readonly<Record<string, string>>>>;
  /** Image URLs by path relative to the content root (`<section>/images/<name>.png`). */
  images?: Readonly<Record<string, string>>;
}

const extensions: DocsExtension[] = [];
const listeners = new Set<() => void>();

/**
 * Adds pages to the docs. Call it at startup, before the docs are first shown: the docs are
 * rebuilt with them, but a language already loaded keeps the translations it had.
 */
export function extendDocs(ext: DocsExtension): void {
  extensions.push(ext);
  for (const l of listeners) l();
}

const pending: Array<() => Promise<DocsExtension>> = [];

/**
 * Adds pages loaded only when the docs first show (`loadDocsExtensions`): call it at startup,
 * so the pages (all their languages' Markdown) stay out of memory until someone opens the docs.
 */
export function extendDocsLazily(load: () => Promise<DocsExtension>): void {
  pending.push(load);
}

/** Loads the pages added with `extendDocsLazily` (the docs await it before they first show). */
export async function loadDocsExtensions(): Promise<void> {
  while (pending.length > 0) {
    const loads = pending.splice(0);
    for (const ext of await Promise.all(loads.map((load) => load()))) extendDocs(ext);
  }
}

/** Every extension, in the order they were added. */
export function docsExtensions(): readonly DocsExtension[] {
  return extensions;
}

/** Calls `f` when an extension is added (the docs drop what they built). */
export function onDocsExtended(f: () => void): void {
  listeners.add(f);
}
