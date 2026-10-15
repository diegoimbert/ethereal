// The French docs (docs-site/content-fr): a page there replaces its English one; the others
// stay English (docs/LOCALIZATION.md "Docs"). Its own chunk, loaded only in French.
const ROOT = "../../../../docs-site/content-fr/";

const markdown = import.meta.glob("../../../../docs-site/content-fr/**/*.md", { query: "?raw", import: "default", eager: true }) as Record<string, string>;

/** `{ "<section>/<page>.md": source }`. */
export const files: Record<string, string> = Object.fromEntries(Object.entries(markdown).map(([k, v]) => [k.slice(ROOT.length), v]));
