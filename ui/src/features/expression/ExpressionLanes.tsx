/**
 * The lane area under the piano roll: a lane picker (Velocity, the clip's expression lanes,
 * common controllers, any CC, per-note pressure) and the chosen lane. Velocity is passed in
 * by the piano roll (`velocity`), everything else is drawn here.
 */

import { useState, type ReactNode, type Ref } from "react";
import type { Clip, ExpressionKind, Note } from "@/generated";
import { Button, NumberField, Select, type SelectOption } from "@/kit";
import type { TimelineViewport } from "@/timeline";
import { cmd, useTransport } from "@/transport";
import { ExpressionLaneView } from "./ExpressionLaneView";
import { useExpressionLanesOf } from "./hooks";
import { choiceKey, COMMON_KINDS, NOTE_KINDS, OTHER_CC, type LaneChoice } from "./choices";
import { kindKey, kindLabel, MAX_EXPRESSION_CC, noteKindLabel, sameKind } from "./model";
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
}

export function ExpressionLanes({ clip, notes, vp, widthPx, height, stepBeats, labelWidth, velocity, bodyRef }: ExpressionLanesProps) {
  const transport = useTransport();
  const lanes = useExpressionLanesOf(clip.id);
  const [choice, setChoice] = useState<LaneChoice>({ type: "Velocity" });
  const [otherCc, setOtherCc] = useState(false);

  const kinds: ExpressionKind[] = [...COMMON_KINDS];
  for (const l of [...lanes].sort((a, b) => kindKey(a.kind).localeCompare(kindKey(b.kind), undefined, { numeric: true }))) {
    if (!kinds.some((k) => sameKind(k, l.kind))) kinds.push(l.kind);
  }
  if (choice.type === "Lane" && !kinds.some((k) => sameKind(k, choice.kind))) kinds.push(choice.kind);
  const used = (k: ExpressionKind) => lanes.some((l) => sameKind(l.kind, k) && l.points.length > 0);

  const options: SelectOption<string>[] = [
    { value: "velocity", label: "Velocity", group: "Notes" },
    ...NOTE_KINDS.map((k) => ({ value: `note:${k}`, label: noteKindLabel(k), group: "Notes" })),
    ...kinds.map((k) => ({ value: kindKey(k), label: used(k) ? `${kindLabel(k)} •` : kindLabel(k), group: "Clip expression" })),
    { value: OTHER_CC, label: "Other CC…", group: "Clip expression" },
  ];

  const pick = (value: string) => {
    if (value === OTHER_CC) {
      setOtherCc(true);
      setChoice({ type: "Lane", kind: { type: "Cc", controller: choice.type === "Lane" && choice.kind.type === "Cc" ? choice.kind.controller : 2 } });
      return;
    }
    setOtherCc(false);
    if (value === "velocity") return setChoice({ type: "Velocity" });
    const note = NOTE_KINDS.find((k) => value === `note:${k}`);
    if (note) return setChoice({ type: "Note", kind: note });
    const kind = kinds.find((k) => kindKey(k) === value);
    if (kind) setChoice({ type: "Lane", kind });
  };

  const lane = choice.type === "Lane" ? lanes.find((l) => sameKind(l.kind, choice.kind)) : undefined;

  return (
    <div className="eth-expr" data-testid="expression-lanes">
      <div className="eth-expr__bar">
        <Select size="sm" aria-label="Lane" value={choiceKey(choice)} options={options} onChange={pick} data-testid="expression-lane-picker" />
        {otherCc && choice.type === "Lane" && choice.kind.type === "Cc" && (
          <NumberField
            size="sm"
            aria-label="Controller number"
            min={0}
            max={MAX_EXPRESSION_CC}
            value={choice.kind.controller}
            onChange={(controller) => setChoice({ type: "Lane", kind: { type: "Cc", controller } })}
          />
        )}
        {lane && (
          <Button
            size="sm"
            title={`Remove the ${kindLabel(lane.kind)} lane`}
            onClick={() => {
              transport
                .send(cmd("Expression", { type: "RemoveLane", id: lane.id }))
                .catch((err: unknown) => console.warn("[expression] command failed:", err));
            }}
          >
            Remove lane
          </Button>
        )}
      </div>
      <div className="eth-pr__lane">
        <div className="eth-pr__lane-label" style={{ width: labelWidth }}>
          {choice.type === "Velocity" ? "Velocity" : choice.type === "Lane" ? kindLabel(choice.kind) : noteKindLabel(choice.kind)}
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
          {choice.type === "Note" && <NoteExpressionLaneView kind={choice.kind} notes={notes} vp={vp} widthPx={widthPx} height={height} />}
        </div>
      </div>
    </div>
  );
}
