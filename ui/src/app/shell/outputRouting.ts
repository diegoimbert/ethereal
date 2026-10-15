/** Track output routing choices (the inspector's Output select). */

import type { Track, TrackId, TrackOutput } from "@/generated";
import { childrenOf, isWithin, type Tracks } from "@/features/groups";
import { t } from "@/i18n";

/**
 * Tracks `track` may output to: group and return tracks, except itself and (for a group)
 * its own descendants, which would form a cycle.
 */
export function outputTargets(tracks: Tracks, track: Track): Track[] {
  const out: Track[] = [];
  const visit = (parent: TrackId | null) => {
    for (const t of childrenOf(tracks, parent)) {
      if ((t.kind === "Group" || t.kind === "Return") && !isWithin(tracks, t.id, track.id)) out.push(t);
      if (t.kind === "Group") visit(t.id);
    }
  };
  visit(null);
  return out;
}

/** Label of `TrackOutput::Default` for this track: its parent group bus, else master. */
export function defaultOutputLabel(tracks: Tracks, track: Track): string {
  const parent = track.parent ? tracks[track.parent] : undefined;
  return parent ? t("shell.inspector.output_group", { name: parent.name }) : t("shell.inspector.output_master");
}

/** Output `Select` value encoding of a `TrackOutput`. */
export function outputValue(output: TrackOutput): string {
  switch (output.type) {
    case "Default":
      return "default";
    case "None":
      return "none";
    case "Track":
      return `track:${output.track}`;
  }
}

export function parseOutputValue(value: string): TrackOutput {
  if (value === "none") return { type: "None" };
  if (value.startsWith("track:")) return { type: "Track", track: value.slice("track:".length) };
  return { type: "Default" };
}
