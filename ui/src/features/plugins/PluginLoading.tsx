/**
 * Plugins loading: a big project's plugin instances are created a few per engine tick
 * (`PluginEvent::Loading` lists those still to load). Each track's header shows how far its
 * own plugins are, until all are loaded (the transport bar shows the whole load's progress).
 */
import { useMemo } from "react";
import type { DeviceId, Project, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { useT } from "@/i18n";
import { usePluginEvents, usePluginStore } from "./pluginStore";

/** `"loaded/total"` of the plugin devices (of `track`, or all) while some are pending, else null. */
function progressOf(project: Project | null, pending: ReadonlySet<DeviceId>, track?: TrackId): string | null {
  if (pending.size === 0 || !project) return null;
  let total = 0;
  let left = 0;
  for (const d of Object.values(project.devices)) {
    if (d.kind.type !== "Plugin" || (track !== undefined && d.track !== track)) continue;
    total += 1;
    if (pending.has(d.id)) left += 1;
  }
  return left > 0 ? `${total - left}/${total}` : null;
}

/** `[loaded, total]` of the plugin devices (of `track`, or the project's) while some load. */
function useLoadProgress(track?: TrackId): [number, number] | null {
  usePluginEvents();
  const loading = usePluginStore((s) => s.loading);
  const pending = useMemo(() => new Set(loading), [loading]);
  const key = useProjectStore((s) => progressOf(s.project, pending, track));
  if (key === null) return null;
  const [done, total] = key.split("/").map(Number) as [number, number];
  return [done, total];
}

/** Along the bottom of a track's header card: how many of its plugins are loaded. */
export function TrackPluginLoading({ track }: { track: TrackId }) {
  const t = useT();
  const progress = useLoadProgress(track);
  if (!progress) return null;
  const [done, total] = progress;
  return (
    <span
      className="eth-plugin-loading"
      role="progressbar"
      aria-label={t("plugins.loading.track", { done, total })}
      title={t("plugins.loading.track", { done, total })}
      aria-valuemin={0}
      aria-valuemax={total}
      aria-valuenow={done}
    >
      <span className="eth-plugin-loading__fill" style={{ transform: `scaleX(${done / total})` }} />
    </span>
  );
}
