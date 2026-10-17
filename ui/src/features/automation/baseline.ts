/**
 * The current value of an automation target and the empty-lane baseline.
 *
 * A lane without points doesn't drive its target: the engine plays the document value (the
 * knob, fader or switch the device / mixer UI shows). The lane draws that value as a grayed,
 * dashed line across its width, and the first point created by a double-click takes it, so
 * playback doesn't jump when the lane starts driving the target.
 */

import type { AutomationTarget, ParamInfo, Project } from "@/generated";
import { paramToNormalized } from "@/domain/paramScale";
import { useProjectStore } from "@/state";

/** The project tables `targetValue` reads. */
export type ValueTables = Pick<Project, "tracks" | "sends" | "devices">;

/**
 * The document (static) value of `target`, normalized through `info` (`null` when its entity
 * is gone). Clip envelopes (gain, transposition) are offsets added to the clip: neutral is
 * the param's default (0 dB, 0 st).
 */
export function targetValue(project: ValueTables, target: AutomationTarget, info: ParamInfo): number | null {
  const plain = targetPlain(project, target, info);
  return plain === null || !Number.isFinite(plain) ? null : paramToNormalized(info, plain);
}

function targetPlain(project: ValueTables, target: AutomationTarget, info: ParamInfo): number | null {
  switch (target.type) {
    case "TrackVolume":
      return project.tracks[target.track]?.mixer.volume ?? null;
    case "TrackPan":
      return project.tracks[target.track]?.mixer.pan ?? null;
    case "SendLevel":
      return project.sends[target.send]?.level ?? null;
    case "DeviceParam": {
      const d = project.devices[target.device];
      return d ? (d.params[target.param] ?? info.default) : null;
    }
    case "DeviceEnabled": {
      const d = project.devices[target.device];
      return d ? (d.enabled ? info.max : info.min) : null;
    }
    case "ClipGain":
    case "ClipTranspose":
      return info.default;
  }
}

/** What an empty lane draws: the current value while the lane has no points, else nothing. */
export function emptyLaneBaseline(pointCount: number, current: number | null): number | null {
  return pointCount === 0 ? current : null;
}

/**
 * The live document value of `target` (normalized), re-rendering when it changes: a knob
 * move, a collaborator's edit, an undo.
 */
export function useTargetValue(target: AutomationTarget, info: ParamInfo): number | null {
  return useProjectStore((s) => (s.project ? targetValue(s.project, target, info) : null));
}
