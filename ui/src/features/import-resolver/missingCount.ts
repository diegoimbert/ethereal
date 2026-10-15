/** How many missing items a project has, as the resolver dialog groups them. */
import { useContext } from "react";
import type { Project } from "@/generated";
import { useMediaRefs } from "@/features/media-refs/store";
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { catalogNow, isMissingPlugin } from "./catalog";

/** Missing items as the dialog groups them: plugins (by format and id), devices (by kind), audio files. */
export function missingCount(project: Project | null, transport: EngineTransport | null | undefined, missingMedia: number): number {
  if (!project) return 0;
  const catalog = catalogNow(transport);
  const groups = new Set<string>();
  for (const d of Object.values(project.devices)) {
    if (d.kind.type === "Missing") groups.add(`d:${d.kind.missing.source_id}`);
    else if (d.kind.type === "Plugin" && isMissingPlugin(d, catalog)) groups.add(`p:${d.kind.plugin.format}:${d.kind.plugin.plugin_id.toUpperCase()}`);
  }
  return groups.size + missingMedia;
}

/** How many missing items the open project has (kept current as it changes). */
export function useMissingCount(): number {
  const transport = useContext(TransportContext)?.transport;
  const project = useProjectStore((s) => s.project);
  const media = useMediaRefs((s) => s.missing.size);
  return missingCount(project, transport, media);
}
