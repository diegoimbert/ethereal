// The tab title follows the open project: `ethereal - <name>`, `• ` in front while it
// has unsaved changes; the shared project's name in the viewer (`/s/<id>`); plain `ethereal`
// on the project picker (at launch it shows over the launch draft: not what the user opened).
// Renames (here or a collaborator's), switches and Back / Forward (`/p/<slug>/<id>`) all land
// in the project store, so the title follows them.
import { useProjectStore, useViewer } from "@/state";
import { useProjectScreen } from "@/features/project/screenStore";
import { useTabTitle } from "./title";

export { BRAND, setTabTitle, tabTitle, useTabTitle } from "./title";

/** Mounted once, at the app shell. */
export function useDocumentTitle(): void {
  const name = useProjectStore((s) => s.project?.settings.name ?? null);
  const dirty = useProjectStore((s) => s.dirty);
  const viewer = useViewer();
  const picking = useProjectScreen((s) => s.open && s.mode !== "saveAs");
  // The viewer never saves: its name is the share's, and nothing is ever unsaved.
  const shown = viewer ? viewer.name || name : picking ? null : name;
  useTabTitle(shown, !viewer && shown !== null && dirty);
}
