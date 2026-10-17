// The open project in the address bar (web): `/p/<slug>/<id>`. The id finds the project; the
// slug (the project's name, normalized) is only there so people recognize their projects in
// the browser's history. Opening another project pushes a history entry (Back / Forward open
// the project in the URL), a rename or a stale slug replaces it, and no project (or the
// launch draft behind the picker) is `/`. At launch a `/p/…/<id>` URL opens that project
// (`launch.ts`).
//
// Query parameters and the fragment are kept. The desktop app has no address bar: nothing
// here runs there.
import { contributions } from "@/ext";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import type { MessageKey } from "@/i18n";
import { isDraft } from "./drafts";
import { openProject } from "./actions";
import { guardLeave } from "./leaveGuard";
import { useProjectScreen } from "./screenStore";

/** Longest slug (characters). */
export const SLUG_MAX = 60;

/** Shown on the project picker when the URL's project is not stored in this browser. */
export const MISSING_PROJECT_NOTICE = "project.notice.missing" satisfies MessageKey;

// Latin letters NFKD doesn't decompose into ASCII.
const FOLD: Record<string, string> = { ß: "ss", æ: "ae", œ: "oe", ø: "o", đ: "d", ð: "d", ł: "l", þ: "th", ı: "i" };

/**
 * A project name as a URL slug: lowercase ASCII (accents stripped), every run of other
 * characters one `-`, no leading or trailing `-`, at most `SLUG_MAX` characters; `untitled`
 * when nothing is left (an emoji or CJK-only name).
 */
export function projectSlug(name: string): string {
  const ascii = name
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[ßæœøđðłþı]/g, (c) => FOLD[c] ?? c);
  const slug = ascii
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, SLUG_MAX)
    .replace(/-+$/, "");
  return slug || "untitled";
}

/** The path of a project: `/p/<slug>/<id>`. */
export function projectPath(name: string, id: string): string {
  return `/p/${projectSlug(name)}/${encodeURIComponent(id)}`;
}

/** The project id in a `/p/<slug>/<id>` path (or `/p/<id>`, no slug), else `null`. */
export function parseProjectPath(pathname: string): { slug: string | null; id: string } | null {
  const m = /^\/p\/(?:([^/]*)\/)?([^/]+)\/?$/.exec(pathname);
  if (!m) return null;
  try {
    const id = decodeURIComponent(m[2]!);
    return id ? { slug: m[1] ?? null, id } : null;
  } catch {
    return null;
  }
}

/** Routes owned by someone else (the docs site, an extension's `web.route`s): never rewritten. */
function foreignRoute(pathname: string): boolean {
  return pathname.startsWith("/docs") || contributions("project.foreignRoute").some((owns) => owns(pathname));
}

/** The address bar reflects the project on this host (the browser; not the desktop app). */
export function urlSyncEnabled(transport: Pick<EngineTransport, "kind"> | null | undefined): boolean {
  return !!transport && transport.kind !== "tauri" && typeof window !== "undefined" && typeof history !== "undefined";
}

/** The project id in the address bar (launch), if the URL is a project's. */
export function urlProjectId(transport: Pick<EngineTransport, "kind"> | null | undefined): string | null {
  if (!urlSyncEnabled(transport)) return null;
  return parseProjectPath(location.pathname)?.id ?? null;
}

function go(path: string, push: boolean): void {
  const url = path + location.search + location.hash;
  if (push) history.pushState(null, "", url);
  else history.replaceState(history.state, "", url);
}

/** The URL's project is not stored here: the picker says so and the URL becomes `/`. */
export function showMissingProject(): void {
  if (!foreignRoute(location.pathname)) go("/", false);
  useProjectScreen.getState().show();
  useProjectScreen.setState({ notice: MISSING_PROJECT_NOTICE });
}

/** Is project `id` stored on this engine? */
export async function projectStored(transport: EngineTransport, id: string): Promise<boolean> {
  const list = await transport.send(cmd("Project", { type: "List" }));
  return list.type === "Projects" && list.projects.some((p) => p.id === id);
}

/** Where the address bar should be for the current state. */
export function targetPath(): string {
  const project = useProjectStore.getState().project;
  if (!project) return "/";
  // The launch draft behind the picker (or the screen an extension shows instead): nothing
  // picked yet. An extension whose screen comes or goes calls `syncProjectUrl`.
  if (isDraft(project.id) && (useProjectScreen.getState().open || contributions("project.launchHeld").some((held) => held()))) return "/";
  return projectPath(project.settings.name, project.id);
}

/**
 * Bring the address bar in line with the open project: a different project pushes a history
 * entry (`replace` forces replacing), the same one (a rename, a stale slug) replaces it.
 */
export function syncProjectUrl({ replace = false }: { replace?: boolean } = {}): void {
  const current = location.pathname;
  if (foreignRoute(current)) return;
  const target = targetPath();
  if (target === current) return;
  const from = parseProjectPath(current);
  const to = parseProjectPath(target);
  go(target, !replace && !!from && !!to && from.id !== to.id);
}

/**
 * Keep the address bar on the open project and open the URL's project on Back / Forward.
 * Waits for launch (`launching`): until then the URL is the one to open. Returns the cleanup.
 */
export function bindProjectUrl(transport: EngineTransport): () => void {
  if (!urlSyncEnabled(transport)) return () => undefined;
  // Back / Forward to a project that is opening: the URL is already right, don't touch it.
  let navigating: string | null = null;
  let navTimer: ReturnType<typeof setTimeout> | undefined;
  const settle = () => {
    navigating = null;
    clearTimeout(navTimer);
    syncProjectUrl({ replace: true });
  };
  const sync = () => {
    if (navigating !== null || useProjectScreen.getState().launching) return;
    syncProjectUrl();
  };

  const key = () => {
    const p = useProjectStore.getState().project;
    return p ? `${p.id}\n${p.settings.name}` : "";
  };
  let lastKey = key();
  const offProject = useProjectStore.subscribe(() => {
    const k = key();
    if (k === lastKey) return;
    lastKey = k;
    if (navigating !== null && useProjectStore.getState().project?.id === navigating) settle();
    else sync();
  });
  const offScreen = useProjectScreen.subscribe((s, prev) => {
    if (s.launching !== prev.launching || s.open !== prev.open) sync();
  });

  const onPop = () => {
    const wanted = parseProjectPath(location.pathname);
    if (!wanted || wanted.id === useProjectStore.getState().project?.id || useProjectScreen.getState().launching) {
      sync();
      return;
    }
    const id = wanted.id;
    navigating = id;
    clearTimeout(navTimer);
    // Never stuck: an open that never shows up hands the URL back to the open project.
    navTimer = setTimeout(() => navigating === id && settle(), 15_000);
    void (async () => {
      try {
        if (!(await projectStored(transport, id))) {
          navigating = null;
          showMissingProject();
          return;
        }
        let opened!: () => void;
        const done = new Promise<void>((r) => (opened = r));
        const open = () =>
          openProject(transport, id)
            .catch(() => undefined)
            .finally(opened);
        if (!(await guardLeave(open))) return settle();
        await done;
        if (navigating === id && useProjectStore.getState().project?.id !== id) {
          // The engine refused (or the store follows later: the subscription settles it).
          setTimeout(() => navigating === id && settle(), 1000);
        } else if (navigating === id) settle();
      } catch {
        if (navigating === id) settle();
      }
    })();
  };
  window.addEventListener("popstate", onPop);
  sync();
  return () => {
    clearTimeout(navTimer);
    offProject();
    offScreen();
    window.removeEventListener("popstate", onPop);
  };
}
