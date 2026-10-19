/**
 * The arranger's clips as text, for screen readers: the clip layer's canvas is `aria-hidden`
 * (it draws pixels, not elements), so each track's clips are listed here (name, kind, start
 * and end) next to the selection announcements (`SelectionAnnouncer`). Visually hidden.
 */

import { useMemo } from "react";
import type { Clip } from "@/generated";
import { useT } from "@/i18n";
import { defaultClipName, tracksOrdered, useProjectStore } from "@/state";
import { formatBarBeat, useTempoMap } from "@/timeline";
import { endOf, isArrangementClip, startOf } from "../clipTime";

export function ClipSummary() {
  const t = useT();
  const project = useProjectStore((s) => s.project);
  const tempo = useTempoMap();
  const groups = useMemo(() => {
    if (!project) return [];
    const byTrack = new Map<string, Clip[]>();
    for (const c of Object.values(project.clips)) {
      if (!isArrangementClip(c)) continue;
      const list = byTrack.get(c.track);
      if (list) list.push(c);
      else byTrack.set(c.track, [c]);
    }
    return tracksOrdered(project)
      .filter((track) => byTrack.has(track.id))
      .map((track) => ({ track, clips: byTrack.get(track.id)!.sort((a, b) => startOf(a) - startOf(b)) }));
  }, [project]);
  if (!project) return null;
  return (
    <div className="eth-sr-only" data-testid="arranger-clip-summary">
      <ul aria-label={t("arrangement.summary.clips")}>
        {groups.map(({ track, clips }) => (
          <li key={track.id}>
            <ul aria-label={t("arrangement.summary.track", { name: track.name, count: clips.length })}>
              {clips.map((c) => (
                <li key={c.id}>
                  {t(c.content.type === "Midi" ? "arrangement.summary.midi_clip" : "arrangement.summary.audio_clip", {
                    name: c.name || defaultClipName(project, c),
                    start: formatBarBeat(startOf(c), tempo),
                    end: formatBarBeat(endOf(c), tempo),
                    muted: c.muted ? "true" : "false",
                  })}
                </li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
    </div>
  );
}
