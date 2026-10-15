// The docs over the desktop app (`openDocs` in the app): mounted once by the app shell; the
// docs chunk loads only the first time they open.
import { lazy, Suspense } from "react";
import { closeDocs, useDocsOverlay } from "./open";

const DocsOverlay = lazy(() => import("./DocsOverlay"));

export function DocsRoot() {
  const { open, path, anchor, request } = useDocsOverlay();
  if (!open) return null;
  return (
    <Suspense fallback={null}>
      <DocsOverlay key={request} initial={{ path, anchor }} onClose={closeDocs} />
    </Suspense>
  );
}
