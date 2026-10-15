/**
 * Clip envelopes: automation lanes owned by a clip (`AutomationOwner::Clip`). Their times are
 * content-relative beats, so they move, loop and duplicate with the clip. While the clip
 * plays, an enabled envelope drives its target instead of the track's arrangement lane
 * (CONTRACTS.md §4, `ether_core::automation_rt`).
 *
 * The clip editors show one envelope at a time, picked like the track lanes' parameter:
 * - the piano roll offers them in its lane picker (`ExpressionLanes`, group "Envelope · …");
 * - the warp editor shows `ClipEnvelopeStrip` under the waveform.
 *
 * Points are edited with the track lane's editor (`AutomationLaneView`), so every gesture,
 * modifier and menu of the README's "Automation editing" applies. The lane is created with
 * its first point; picking a parameter never touches the document.
 */

import { useMemo, useState } from "react";
import type { AutomationLane, AutomationOwner, Clip } from "@/generated";
import { Button, Select } from "@/kit";
import { DEFAULT_GRID, type GridSetting, type TempoMap, type TimelineViewStore } from "@/timeline";
import { cmd, useTransport } from "@/transport";
import { songToContent } from "@/features/piano-roll/clipTime";
import { AutomationLaneView } from "./AutomationLaneView";
import { sendEdit } from "./gesture";
import { LANE_PAD } from "./geometry";
import { type TargetInfo } from "./params";
import { usePickerActiveOnly } from "./pickerPrefs";
import { ENVELOPE_PREFIX, envelopeKeyOf, useClipEnvelopes } from "./clipEnvelopeTargets";
import { defaultRange } from "./valueAxis";
import { useT } from "@/i18n";

export interface ClipEnvelopeLaneProps {
  clip: Clip;
  /** The envelope's parameter (`undefined`: its device or descriptor is unavailable). */
  target: TargetInfo | undefined;
  lane: AutomationLane | null;
  /** Horizontal viewport of the editor. */
  view: TimelineViewStore;
  /** View beats of content beat 0 (default 0: the editor's axis is the clip's content). */
  offset?: number;
  height: number;
  grid?: GridSetting;
  /** Tempo map of the editor's axis (grid snapping; default: the song's). */
  tempo?: TempoMap;
}

/** One clip envelope: the automation lane editor on the clip's content axis. */
export function ClipEnvelopeLane({ clip, target, lane, view, offset = 0, height, grid = DEFAULT_GRID, tempo }: ClipEnvelopeLaneProps) {
  const t = useT();
  const owner: AutomationOwner = useMemo(() => ({ type: "Clip", clip: clip.id }), [clip.id]);
  const info = target?.info;
  const range = useMemo(() => (info ? defaultRange(info, height - 2 * LANE_PAD) : undefined), [info, height]);
  // Paste at the edit cursor: the content position playing there (else the content start).
  const cursorToLane = (song: number) => songToContent(clip, song) ?? clip.offset;
  if (!target) {
    return (
      <div className="eth-auto-clip__missing" data-testid="clip-envelope-missing" style={{ height }}>
        {t("automation.lane.unavailable")}
      </div>
    );
  }
  return (
    <div className="eth-auto-clip__lane" data-testid="clip-envelope" data-target={target.key} style={{ height }}>
      <AutomationLaneView
        lane={lane}
        owner={owner}
        target={target.target}
        info={target.info}
        view={view}
        offset={offset}
        height={height}
        range={range}
        grid={grid}
        cursorToLane={cursorToLane}
        tempo={tempo}
        label={t("automation.envelope.label", { name: target.name })}
      />
    </div>
  );
}

/** Enable toggle and delete button of a clip envelope (nothing without a lane). */
export function ClipEnvelopeButtons({ lane, name }: { lane: AutomationLane | null; name: string }) {
  const transport = useTransport();
  const t = useT();
  if (!lane) return null;
  return (
    <>
      <Button
        size="sm"
        active={lane.enabled}
        aria-pressed={lane.enabled}
        data-testid="clip-envelope-enabled"
        title={lane.enabled ? t("automation.envelope.disable", { name }) : t("automation.envelope.enable", { name })}
        onClick={() => void sendEdit(transport, cmd("Automation", { type: "SetLaneEnabled", id: lane.id, enabled: !lane.enabled }))}
      >
        {lane.enabled ? t("automation.envelope.on") : t("common.off")}
      </Button>
      <Button
        size="sm"
        data-testid="clip-envelope-delete"
        title={t("automation.envelope.delete_title", { name })}
        onClick={() => void sendEdit(transport, cmd("Automation", { type: "DeleteLane", id: lane.id }))}
      >
        {t("automation.envelope.delete")}
      </Button>
    </>
  );
}

export interface ClipEnvelopeStripProps {
  clip: Clip;
  view: TimelineViewStore;
  offset?: number;
  height: number;
  /** Width of the label column left of the lane (px). */
  labelWidth?: number;
  grid?: GridSetting;
  tempo?: TempoMap;
}

/**
 * A picker and one clip envelope (the warp editor's envelope area). Opens on the first
 * envelope that holds points, else on the track volume.
 */
export function ClipEnvelopeStrip({ clip, view, offset = 0, height, labelWidth = 0, grid, tempo }: ClipEnvelopeStripProps) {
  const t = useT();
  const { targets, lanes, options } = useClipEnvelopes(clip);
  const [activeOnly, setActiveOnly] = usePickerActiveOnly();
  const [picked, setPicked] = useState<string | null>(null);
  const first = options[0] ? envelopeKeyOf(options[0].value) : null;
  const key = picked ?? first;
  const target = targets.find((x) => x.key === key);
  const lane = key ? (lanes.get(key) ?? null) : null;
  // A lane whose parameter isn't listed (device gone, descriptor not loaded) stays editable
  // by name only: show it as unavailable.
  const value = key ? ENVELOPE_PREFIX + key : "";
  const shown = target || !lane ? options : [...options, { value, label: t("automation.lane.unavailable"), group: t("automation.envelope.in_clip") }];
  return (
    <div className="eth-auto-clip" data-testid="clip-envelopes">
      <div className="eth-auto-clip__bar">
        <span className="eth-auto-clip__title">{t("automation.envelope.title")}</span>
        <Select
          size="sm"
          aria-label={t("automation.envelope.picker")}
          data-testid="clip-envelope-picker"
          value={value}
          placeholder={t("automation.envelope.parameter")}
          options={shown}
          searchable
          activeFilter={{ value: activeOnly, onChange: setActiveOnly }}
          onChange={(v) => setPicked(envelopeKeyOf(v))}
        />
        <ClipEnvelopeButtons lane={lane} name={target?.name ?? t("automation.envelope.parameter_unnamed")} />
      </div>
      <div className="eth-auto-clip__row">
        {labelWidth > 0 && <div className="eth-auto-clip__label" style={{ width: labelWidth }} />}
        <div className="eth-auto-clip__body">
          {key && <ClipEnvelopeLane key={key} clip={clip} target={target} lane={lane} view={view} offset={offset} height={height} grid={grid} tempo={tempo} />}
        </div>
      </div>
    </div>
  );
}
