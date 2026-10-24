/**
 * The lane area under the piano roll: a lane picker (Velocity, the clip's expression lanes,
 * common controllers, any CC, per-note pressure, the clip's envelopes) and the chosen lane.
 * Velocity is passed in by the piano roll (`velocity`), everything else is drawn here.
 */

import clsx from "clsx";
import { useState, type ReactNode, type Ref } from "react";
import type { Clip, ExpressionKind, Note } from "@/generated";
import { Button, NumberField, Select, type SelectOption } from "@/kit";
import type { TempoMap, TimelineViewport, TimelineViewStore } from "@/timeline";
import { ClipEnvelopeButtons, ClipEnvelopeLane, envelopeKeyOf, NotesBackdrop, targetNameIsUserText, useClipEnvelopes } from "@/features/automation";
import { cmd, useTransport } from "@/transport";
import { confirmRemoveLane } from "./confirm";
import { ExpressionLaneView } from "./ExpressionLaneView";
import { useExpressionLanesOf } from "./hooks";
import { choiceKey, COMMON_KINDS, NOTE_KINDS, OTHER_CC, type LaneChoice } from "./choices";
import { kindKey, MAX_EXPRESSION_CC, sameKind, type Range } from "@/domain/expression";
import { t as tr, useT } from "@/i18n";
import { kindText, noteKindText } from "./labels";
import { pitchWindow } from "@/domain/mpe";
import { useTrack } from "@/state";
import { NoteExpressionLaneView } from "./NoteExpressionLaneView";
import "./expression.css";

export interface ExpressionLanesProps {
  clip: Clip;
  notes: ReadonlyArray<Note>;
  vp: TimelineViewport;
  widthPx: number;
  height: number;
  stepBeats: number;
  labelWidth: number;
  /** The velocity lane (owned by the piano roll). */
  velocity: ReactNode;
  /** The lane body (wheel zoom / middle-button pan are attached by the piano roll). */
  bodyRef?: Ref<HTMLDivElement>;
  /** The editor's view store (clip envelopes use the automation lane editor). */
  view: TimelineViewStore;
  /** View beats of the clip's content beat 0 (multi-clip editing: the focus shift). */
  envelopeOffset?: number;
  /** Tempo map of the editor's axis. */
  tempo?: TempoMap;
}

export function ExpressionLanes(props: ExpressionLanesProps) {
  const { clip, notes, vp, widthPx, height, stepBeats, labelWidth, velocity, bodyRef, view, envelopeOffset = 0, tempo } = props;
  const t = useT();
  const transport = useTransport();
  const lanes = useExpressionLanesOf(clip.id);
  const envelopes = useClipEnvelopes(clip);
  const [choice, setChoice] = useState<LaneChoice>({ type: "Velocity" });
  const [otherCc, setOtherCc] = useState(false);
  // `mpe`: the Pitch lane spans the track's per-note bend range.
  const pitchSpan = pitchWindow(useTrack(clip.track)?.mpe);
  const pitchRange: Range = [-pitchSpan, pitchSpan];

  const kinds: ExpressionKind[] = [...COMMON_KINDS];
  for (const l of [...lanes].sort((a, b) => kindKey(a.kind).localeCompare(kindKey(b.kind), undefined, { numeric: true }))) {
    if (!kinds.some((k) => sameKind(k, l.kind))) kinds.push(l.kind);
  }
  if (choice.type === "Lane" && !kinds.some((k) => sameKind(k, choice.kind))) kinds.push(choice.kind);
  const used = (k: ExpressionKind) => lanes.some((l) => sameKind(l.kind, k) && l.points.length > 0);

  const options: SelectOption<string>[] = [
    { value: "velocity", label: t("expression.picker.velocity"), group: t("expression.picker.notes") },
    ...NOTE_KINDS.map((k) => ({ value: `note:${k}`, label: noteKindText(k), group: t("expression.picker.notes") })),
    ...kinds.map((k) => ({ value: kindKey(k), label: used(k) ? `${kindText(k)} •` : kindText(k), group: t("expression.picker.clip") })),
    { value: OTHER_CC, label: t("expression.picker.other_cc"), group: t("expression.picker.clip") },
    ...envelopes.options,
  ];
  const envelope = choice.type === "Envelope" ? envelopes.targets.find((x) => x.key === choice.key) : undefined;
  const envelopeLane = choice.type === "Envelope" ? (envelopes.lanes.get(choice.key) ?? null) : null;
  // A picked envelope whose parameter went away (device deleted) stays selected, unnamed.
  if (choice.type === "Envelope" && !options.some((o) => o.value === choiceKey(choice))) {
    options.push({ value: choiceKey(choice), label: t("automation.lane.unavailable"), group: t("automation.envelope.in_clip") });
  }

  const pick = (value: string) => {
    if (value === OTHER_CC) {
      setOtherCc(true);
      setChoice({ type: "Lane", kind: { type: "Cc", controller: choice.type === "Lane" && choice.kind.type === "Cc" ? choice.kind.controller : 2 } });
      return;
    }
    setOtherCc(false);
    if (value === "velocity") return setChoice({ type: "Velocity" });
    const env = envelopeKeyOf(value);
    if (env !== null) return setChoice({ type: "Envelope", key: env });
    const note = NOTE_KINDS.find((k) => value === `note:${k}`);
    if (note) return setChoice({ type: "Note", kind: note });
    const kind = kinds.find((k) => kindKey(k) === value);
    if (kind) setChoice({ type: "Lane", kind });
  };

  const lane = choice.type === "Lane" ? lanes.find((l) => sameKind(l.kind, choice.kind)) : undefined;

  return (
    <div className="eth-expr" data-testid="expression-lanes">
      <div className="eth-expr__bar">
        <Select
          size="sm"
          aria-label={t("expression.picker.label")}
          value={choiceKey(choice)}
          options={options}
          onChange={pick}
          data-testid="expression-lane-picker"
        />
        {otherCc && choice.type === "Lane" && choice.kind.type === "Cc" && (
          <NumberField
            size="sm"
            aria-label={t("expression.picker.controller")}
            min={0}
            max={MAX_EXPRESSION_CC}
            value={choice.kind.controller}
            onChange={(controller) => setChoice({ type: "Lane", kind: { type: "Cc", controller } })}
          />
        )}
        {lane && (
          <Button
            size="sm"
            title={t("expression.lane.remove_title", { lane: kindText(lane.kind) })}
            onClick={() =>
              confirmRemoveLane(kindText(lane.kind), lane.points.length, () => {
                transport
                  .send(cmd("Expression", { type: "RemoveLane", id: lane.id }))
                  .catch((err: unknown) => console.warn("[expression] command failed:", err));
              })
            }
          >
            {t("expression.lane.remove")}
          </Button>
        )}
        {choice.type === "Envelope" && <ClipEnvelopeButtons lane={envelopeLane} name={envelope?.name ?? "parameter"} />}
      </div>
      <div className="eth-pr__lane">
        <div className={clsx("eth-pr__lane-label", envelope && targetNameIsUserText(envelope) && "eth-user-text")} style={{ width: labelWidth }}>
          {laneLabel(choice, envelope?.name)}
        </div>
        <div ref={bodyRef} className="eth-pr__lane-body">
          {choice.type === "Velocity" && velocity}
          {choice.type === "Lane" && (
            <ExpressionLaneView
              key={kindKey(choice.kind)}
              clip={clip.id}
              kind={choice.kind}
              lane={lane}
              vp={vp}
              widthPx={widthPx}
              height={height}
              stepBeats={stepBeats}
            />
          )}
          {choice.type === "Note" && (
            <NoteExpressionLaneView
              key={choice.kind}
              kind={choice.kind}
              notes={notes}
              vp={vp}
              widthPx={widthPx}
              height={height}
              range={choice.kind === "Pitch" ? pitchRange : undefined}
            />
          )}
          {choice.type === "Envelope" && (
            <ClipEnvelopeLane
              key={choice.key}
              clip={clip}
              target={envelope}
              lane={envelopeLane}
              view={view}
              offset={envelopeOffset}
              height={height}
              grid={{ type: "Fixed", step: { kind: "beats", beats: stepBeats }, triplet: false }}
              tempo={tempo}
              backdrop={<NotesBackdrop notes={notes} vp={vp} width={widthPx} height={height} />}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function laneLabel(choice: LaneChoice, envelopeName: string | undefined): string {
  switch (choice.type) {
    case "Velocity":
      return tr("expression.picker.velocity");
    case "Lane":
      return kindText(choice.kind);
    case "Note":
      return noteKindText(choice.kind);
    case "Envelope":
      return envelopeName ?? tr("automation.envelope.title");
  }
}
