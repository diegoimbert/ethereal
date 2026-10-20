// The user docs: `/docs` on the web (its own bundle, `mount.tsx`), over the app on the
// desktop (`DocsRoot`). Content: docs-site/content (docs-site/README.md).
export { closeDocs, openDocs, useDocsOverlay } from "./open";
export { DocsRoot } from "./DocsRoot";
export { DOCS_SITE_URL, isDocsPage } from "./route";
