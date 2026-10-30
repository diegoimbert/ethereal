/**
 * What uses each media of the project (the media manager): audio clips, built-in devices
 * that load it (samplers, multisampler zones, convolution IRs) and freeze renders. Mirrors
 * `Project::media_users` (engine side), which `MediaRef::Remove` unlinks.
 */

import type { ClipId, DeviceId, MediaId, Project, TrackId } from "@/generated";

export interface MediaUsage {
  clips: ClipId[];
  devices: DeviceId[];
  freezes: TrackId[];
}

const empty = (): MediaUsage => ({ clips: [], devices: [], freezes: [] });

/** Every media of `project`, with what uses it (unused media have empty lists). */
export function mediaUsage(project: Project): Map<MediaId, MediaUsage> {
  const out = new Map<MediaId, MediaUsage>();
  for (const id of Object.keys(project.media)) out.set(id, empty());
  const at = (id: MediaId) => {
    let u = out.get(id);
    if (!u) out.set(id, (u = empty()));
    return u;
  };
  for (const c of Object.values(project.clips)) {
    if (c.content.type === "Audio") at(c.content.media).clips.push(c.id);
  }
  for (const d of Object.values(project.devices)) {
    if (d.kind.type !== "Builtin") continue;
    const b = d.kind.device;
    const used = new Set<MediaId>();
    if (b.type === "Sampler" && b.sample) used.add(b.sample);
    if (b.type === "MultiSampler") for (const z of b.zones) if (z.media) used.add(z.media);
    if (b.type === "ConvolutionReverb" && b.ir?.type === "Media") used.add(b.ir.media);
    for (const m of used) at(m).devices.push(d.id);
  }
  for (const t of Object.values(project.tracks)) {
    if (t.freeze) at(t.freeze.media).freezes.push(t.id);
  }
  return out;
}

export function isUnused(u: MediaUsage | undefined): boolean {
  return !u || (u.clips.length === 0 && u.devices.length === 0 && u.freezes.length === 0);
}
