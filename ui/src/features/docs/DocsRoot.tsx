// The docs over the desktop app (`openDocs` in the app): mounted once by the app shell; the
// docs chunk loads only the first time they open.
import { lazy, Suspense } from "react";
import { loadDocsExtensions } from "./extensions";
import { closeDocs, useDocsOverlay } from "./open";

// With the pages a build adds lazily (`extendDocsLazily`).
const DocsOverlay = lazy(() => loadDocsExtensions().then(() => import("./DocsOverlay")));

export function DocsRoot() {
  const { open, path, anchor, request } = useDocsOverlay();
  if (!open) return null;
  return (
    <Suspense fallback={null}>
      <DocsOverlay key={request} initial={{ path, anchor }} onClose={closeDocs} />
    </Suspense>
  );
}
