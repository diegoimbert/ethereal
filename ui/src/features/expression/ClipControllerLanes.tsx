/**
 * The open MIDI clip's own controller data (CC, pitch bend, channel pressure: its
 * expression lanes) in the editor drawer's automation tab, above the track's automation.
 *
 * One row per controller the clip holds (shown by default, in controller order), plus the
 * ones added with the "+ controller…" picker (empty until drawn; they stay listed while the
 * clip stays open). The rows sit on the tab's song timeline: content beat `b` is drawn at
 * song beat `clip.start - clip.offset + b`, and the time outside the clip is dimmed. Editing
 * is `ExpressionLaneView`'s (pencil, point drag, double-click, right-click), so it is the same
 * data and the same commands as the piano roll's lane area.
 *
 * This is clip-owned data, separate from track and device automation (the rows below it):
 * the section is labelled with the clip's name and moves, loops and duplicates with the clip.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import type { Clip, ExpressionKind } from "@/generated";
import { X } from "lucide-react";
import { Button, IconButton, NumberField, Select, type SelectOption } from "@/kit";
import { DEFAULT_GRID, resolveGrid, stepLength, useTempoMap, useTimelineView, useViewport, type TimelineViewStore } from "@/timeline";
import { cmd, useTransport } from "@/transport";
import { useT } from "@/i18n";
import { clipDisplayName, useClip, useEditedClipId, useProjectStore } from "@/state";
import { kindKey, MAX_EXPRESSION_CC, sameKind } from "@/domain/expression";
import { compareKinds } from "./badge";
import { COMMON_KINDS } from "./choices";
import { confirmRemoveLane } from "./confirm";
import { ExpressionLaneView } from "./ExpressionLaneView";
import { useExpressionLanesOf } from "./hooks";
import { kindText } from "./labels";
import "./expression.css";

/** Height of one controller row (px). */
export const CONTROLLER_LANE_HEIGHT = 56;
const OTHER_CC = "other-cc";
const FALLBACK_STEP = 0.25;

export interface ClipControllerLanesProps {
  /** A MIDI clip. */
  clip: Clip;
  /** The tab's song-time view (its ruler sets the width). */
  view: TimelineViewStore;
  headerWidth: number;
}

export function ClipControllerLanes({ clip, view, headerWidth }: ClipControllerLanesProps) {
  const t = useT();
  const transport = useTransport();
  const lanes = useExpressionLanesOf(clip.id);
  // Kinds added from the picker (no lane until the first stroke), for this clip.
  const [added, setAdded] = useState<{ clip: string; kinds: ExpressionKind[] }>({ clip: clip.id, kinds: [] });
  const addedKinds = added.clip === clip.id ? added.kinds : [];
  const [otherCc, setOtherCc] = useState<number | null>(null);

  const songVp = useViewport(view);
  const widthPx = useTimelineView(view, (s) => s.widthPx);
  const tempo = useTempoMap();
  // A newly opened clip out of view: the view fits it (once per clip, when the width is known).
  const revealed = useRef<string | null>(null);
  useEffect(() => {
    if (widthPx <= 0 || revealed.current === clip.id) return;
    revealed.current = clip.id;
    const { start, end } = view.getState().visibleRange();
    if (clip.start >= end || clip.start + clip.length <= start) view.getState().zoomToRange({ start: clip.start, end: clip.start + clip.length }, 24);
  }, [clip.id, clip.start, clip.length, view, widthPx]);
  const shift = clip.start - clip.offset;
  // The lanes draw on the clip's content axis: the song view shifted to the clip.
  const vp = useMemo(() => ({ pxPerBeat: songVp.pxPerBeat, scrollBeats: songVp.scrollBeats - shift }), [songVp, shift]);
  const sig = tempo.signatureAt(clip.start);
  const step = resolveGrid(DEFAULT_GRID, vp.pxPerBeat, sig);
  const stepBeats = step ? stepLength(step, sig) : FALLBACK_STEP;

  const kinds: ExpressionKind[] = lanes.map((l) => l.kind);
  for (const k of addedKinds) if (!kinds.some((x) => sameKind(x, k))) kinds.push(k);
  kinds.sort(compareKinds);

  const add = (kind: ExpressionKind) => {
    setOtherCc(null);
    if (kinds.some((k) => sameKind(k, kind))) return;
    setAdded({ clip: clip.id, kinds: [...addedKinds, kind] });
  };
  const options: SelectOption<string>[] = [
    ...COMMON_KINDS.filter((k) => !kinds.some((x) => sameKind(x, k))).map((k) => ({ value: kindKey(k), label: kindText(k) })),
    { value: OTHER_CC, label: t("expression.picker.other_cc") },
  ];
  const onPick = (value: string) => {
    if (value === OTHER_CC) return setOtherCc(2);
    const kind = COMMON_KINDS.find((k) => kindKey(k) === value);
    if (kind) add(kind);
  };
  const remove = (kind: ExpressionKind) => {
    setAdded({ clip: clip.id, kinds: addedKinds.filter((k) => !sameKind(k, kind)) });
    const lane = lanes.find((l) => sameKind(l.kind, kind));
    if (!lane) return;
    confirmRemoveLane(kindText(kind), lane.points.length, () => {
      transport.send(cmd("Expression", { type: "RemoveLane", id: lane.id })).catch((err: unknown) => console.warn("[expression] command failed:", err));
    });
  };

  // The clip's span on the lane, in lane px (the rest is dimmed).
  const clipX0 = (clip.start - songVp.scrollBeats) * songVp.pxPerBeat;
  const clipX1 = clipX0 + clip.length * songVp.pxPerBeat;
  const name = useProjectStore((s) => (s.project ? clipDisplayName(s.project, clip) : clip.name));

  return (
    <section className="eth-expr-clip" data-testid="clip-controller-lanes" aria-label={t("expression.clip_lanes.label", { name })}>
      <div className="eth-expr-clip__bar">
        <div className="eth-expr-clip__head" style={{ width: headerWidth }}>
          <span className="eth-expr-clip__title">{t("expression.clip_lanes.title")}</span>
          <span className="eth-expr-clip__clip eth-user-text">{name}</span>
        </div>
        <div className="eth-expr-clip__tools">
          <Select
            size="sm"
            aria-label={t("expression.clip_lanes.add")}
            value=""
            placeholder={t("expression.clip_lanes.add_placeholder")}
            options={options}
            onChange={onPick}
            data-testid="clip-controller-add"
          />
          {otherCc !== null && (
            <>
              <NumberField size="sm" aria-label={t("expression.picker.controller")} min={0} max={MAX_EXPRESSION_CC} value={otherCc} onChange={setOtherCc} />
              <Button size="sm" onClick={() => add({ type: "Cc", controller: otherCc })}>
                {t("expression.clip_lanes.add_cc")}
              </Button>
            </>
          )}
          {kinds.length === 0 && <span className="eth-expr-clip__empty">{t("expression.clip_lanes.empty")}</span>}
        </div>
      </div>
      {kinds.map((kind) => {
        const lane = lanes.find((l) => sameKind(l.kind, kind));
        const text = kindText(kind);
        return (
          <div key={kindKey(kind)} className="eth-expr-clip__row" style={{ height: CONTROLLER_LANE_HEIGHT }} data-kind={kindKey(kind)}>
            <div className="eth-expr-clip__head eth-expr-clip__head--row" style={{ width: headerWidth }}>
              <span className="eth-expr-clip__name">{text}</span>
              <IconButton size="sm" label={t("expression.lane.remove_named", { lane: text })} icon={<X />} onClick={() => remove(kind)} />
            </div>
            <div className="eth-expr-clip__lane">
              <ExpressionLaneView clip={clip.id} kind={kind} lane={lane} vp={vp} widthPx={widthPx} height={CONTROLLER_LANE_HEIGHT} stepBeats={stepBeats} />
              {clipX0 > 0 && <div className="eth-expr-clip__outside" style={{ left: 0, width: Math.min(clipX0, widthPx) }} />}
              {clipX1 < widthPx && <div className="eth-expr-clip__outside" style={{ left: Math.max(0, clipX1), right: 0 }} />}
            </div>
          </div>
        );
      })}
    </section>
  );
}

/** The drawer's open clip's controller lanes, when it is a MIDI clip (else nothing). */
export function EditedClipControllerLanes({ view, headerWidth }: Omit<ClipControllerLanesProps, "clip">) {
  const clip = useClip(useEditedClipId());
  if (!clip || clip.content.type !== "Midi") return null;
  return <ClipControllerLanes clip={clip} view={view} headerWidth={headerWidth} />;
}
