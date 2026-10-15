/**
 * Clip envelopes: which lanes a clip has and what its envelope picker offers (the components
 * are in `ClipEnvelopes.tsx`).
 */

import { useMemo } from "react";
import type { AutomationLane, Clip, ClipId } from "@/generated";
import type { SelectOption } from "@/kit";
import { useProjectStore } from "@/state";
import { CLIP_GAIN_INFO, CLIP_TRANSPOSE_INFO, targetKey, targetNameIsUserText, useTrackTargets, type TargetInfo } from "./params";
import { t as tr } from "@/i18n";

const EMPTY_LANES: Readonly<Record<string, AutomationLane>> = {};
const EMPTY_POINTS: Readonly<Record<string, { lane: string }>> = {};

/** Prefix of the picker values of clip envelopes (the piano roll's picker mixes lane kinds). */
export const ENVELOPE_PREFIX = "env:";

/** Clip envelopes of a clip, by target key. */
export function useClipLanes(clip: ClipId): ReadonlyMap<string, AutomationLane> {
  const lanes = useProjectStore((s) => s.project?.automation_lanes ?? EMPTY_LANES);
  return useMemo(() => {
    const m = new Map<string, AutomationLane>();
    for (const l of Object.values(lanes)) if (l.owner.type === "Clip" && l.owner.clip === clip) m.set(targetKey(l.target), l);
    return m;
  }, [lanes, clip]);
}

/** Lanes (ids) of `lanes` that hold at least one point. */
export function useLanesWithPoints(lanes: ReadonlyMap<string, AutomationLane>): ReadonlySet<string> {
  const points = useProjectStore((s) => s.project?.automation_points ?? EMPTY_POINTS);
  return useMemo(() => {
    const ids = new Set([...lanes.values()].map((l) => l.id));
    const used = new Set<string>();
    for (const p of Object.values(points)) if (ids.has(p.lane)) used.add(p.lane);
    return used;
  }, [lanes, points]);
}

/** Targets of an audio clip's own gain and transposition (modulations of the clip's values), in the current language. */
function audioClipTargets(): TargetInfo[] {
  const gain = tr("automation.target.clip_gain");
  const transposition = tr("automation.target.transposition");
  const group = tr("automation.target.clip");
  return [
    { key: targetKey({ type: "ClipGain" }), target: { type: "ClipGain" }, name: gain, group, info: { ...CLIP_GAIN_INFO, name: gain } },
    {
      key: targetKey({ type: "ClipTranspose" }),
      target: { type: "ClipTranspose" },
      name: transposition,
      group,
      info: { ...CLIP_TRANSPOSE_INFO, name: transposition },
    },
  ];
}

/**
 * What a clip envelope of `clip` can drive: an audio clip's own gain and transposition
 * first, then its track's volume, pan, sends and device params.
 */
export function useClipEnvelopeTargets(clip: Clip): TargetInfo[] {
  const track = useTrackTargets(clip.track);
  const audio = clip.content.type === "Audio";
  // `track` is new on a language switch (`useTrackTargets`), so these follow it.
  return useMemo(() => (audio ? [...audioClipTargets(), ...track] : track), [audio, track]);
}

/**
 * Picker options for the clip's envelopes, values prefixed with [`ENVELOPE_PREFIX`], one
 * group per device ("envelope · mixer", "envelope · Synth"). Parameters whose envelope holds
 * points are marked "•" and listed first, in their own group.
 */
export function clipEnvelopeOptions(
  targets: ReadonlyArray<TargetInfo>,
  lanes: ReadonlyMap<string, AutomationLane>,
  withPoints: ReadonlySet<string>,
): SelectOption<string>[] {
  const used = (t: TargetInfo) => {
    const l = lanes.get(t.key);
    return l !== undefined && withPoints.has(l.id);
  };
  const opt = (t: TargetInfo, group: string): SelectOption<string> => ({
    value: ENVELOPE_PREFIX + t.key,
    label: used(t) ? `${t.name} •` : t.name,
    userText: targetNameIsUserText(t),
    group,
    active: used(t),
  });
  const out = targets.filter(used).map((t) => opt(t, tr("automation.envelope.in_clip")));
  for (const t of targets) if (!used(t)) out.push(opt(t, tr("automation.envelope.group", { group: t.group })));
  return out;
}

/** The target key of a picker value, or `null` when it isn't a clip envelope. */
export function envelopeKeyOf(value: string): string | null {
  return value.startsWith(ENVELOPE_PREFIX) ? value.slice(ENVELOPE_PREFIX.length) : null;
}

/** The clip's envelope lanes plus the picker data, for an editor that hosts one envelope. */
export function useClipEnvelopes(clip: Clip) {
  const targets = useClipEnvelopeTargets(clip);
  const lanes = useClipLanes(clip.id);
  const withPoints = useLanesWithPoints(lanes);
  const options = useMemo(() => clipEnvelopeOptions(targets, lanes, withPoints), [targets, lanes, withPoints]);
  return { targets, lanes, options };
}
