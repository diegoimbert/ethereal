import type { ClipId, SiteId } from "@/generated";
import { peerColor, useCollabStore } from "../store";

/** The peers (not this user) with `clip` open in their piano roll. */
export function useClipEditors(clip: ClipId | null): Array<{ site: SiteId; name: string; color: string }> {
  // A primitive key so the component only re-renders when the answer changes.
  const key = useCollabStore((s) =>
    // collab-social: nobody is shown editing while "Hide users and notes" is on.
    clip === null || s.hideOthers
      ? ""
      : s.peers
          .filter((p) => p.state.editing_clip === clip)
          .map((p) => `${p.site}\u0001${p.name || "Anonymous"}\u0001${peerColor(p.color)}`)
          .join("\u0002"),
  );
  if (!key) return [];
  return key.split("\u0002").map((entry) => {
    const [site, name, color] = entry.split("\u0001") as [SiteId, string, string];
    return { site, name, color };
  });
}
