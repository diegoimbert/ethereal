// The docs' routers: the address bar (`/docs/…`, standalone) or memory (over the desktop app).
import { useEffect, useMemo, useState } from "react";
import type { DocsNav } from "./DocsMarkdown";
import { docsHref, parseDocsUrl, type DocsLocation } from "./route";

export interface DocsRouter extends DocsNav {
  loc: DocsLocation;
}

/** `/docs/…` in the address bar: links push history entries; back / forward work. */
export function useUrlRouter(): DocsRouter {
  const read = (): DocsLocation => parseDocsUrl(location.pathname, location.hash);
  const [loc, setLoc] = useState<DocsLocation>(read);
  useEffect(() => {
    const onPop = () => setLoc(read());
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);
  return useMemo(
    () => ({
      loc,
      href: docsHref,
      go: (path: string, anchor = "") => {
        const url = docsHref(path, anchor);
        if (url !== location.pathname + location.hash) history.pushState(null, "", url);
        setLoc({ path, anchor });
      },
    }),
    [loc],
  );
}

/** In memory (over the desktop app): no address bar. Remount (`key`) to start elsewhere. */
export function useMemoryRouter(initial: DocsLocation): DocsRouter {
  const [loc, setLoc] = useState(initial);
  return useMemo(() => ({ loc, href: docsHref, go: (path: string, anchor = "") => setLoc({ path, anchor }) }), [loc]);
}
