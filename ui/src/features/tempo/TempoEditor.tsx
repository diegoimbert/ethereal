/**
 * Tempo editor (the app shell's "Tempo" detail tab): the project's tempo map on its own
 * ruler, with a time-signature lane and a tempo lane (steps and ramps). The header column
 * edits the selected point precisely. Every edit is one undo step (a drag included).
 */

import { useMemo, useRef, useState } from "react";
import type { TempoCurve, TimeSignature } from "@/generated";
import { Button, NumberField, Select } from "@/kit";
import { playheadStore, useProjectStore } from "@/state";
import { useTransport } from "@/transport";
import { createTimelineViewStore, PlayheadLine, Ruler, useTempoMap, useTimelineWheel } from "@/timeline";
import { sendEdit } from "./gesture";
import { SignatureLane } from "./SignatureLane";
import { TempoLane } from "./TempoLane";
import {
  addSignatureCommand,
  addTempoPointCommand,
  DENOMINATORS,
  editSignatureCommand,
  editTempoPointCommand,
  isAtZero,
  MAX_BPM,
  MIN_BPM,
  removeSignaturesCommand,
  removeTempoPointsCommand,
  snapSignatureTime,
  sortedSignatures,
  sortedTempoPoints,
  tempoTimeTaken,
} from "./tempoCommands";
import "./tempo.css";

const HEADER_WIDTH = 180;

const CURVES: ReadonlyArray<{ value: TempoCurve; label: string }> = [
  { value: "Step", label: "Hold" },
  { value: "Linear", label: "Ramp" },
];
const DENOMINATOR_OPTIONS = DENOMINATORS.map((d) => ({ value: String(d), label: String(d) }));

type Selection = { kind: "tempo" | "signature"; id: string } | null;

export function TempoEditor() {
  const hasProject = useProjectStore((s) => s.project !== null);
  if (!hasProject) {
    return (
      <div className="eth-tempo eth-tempo--empty" data-feature="tempo" data-testid="tempo-editor">
        No project open.
      </div>
    );
  }
  return <TempoEditorBody />;
}

function TempoEditorBody() {
  const transport = useTransport();
  const tempoPoints = useProjectStore((s) => s.project!.tempo_points);
  const timeSignatures = useProjectStore((s) => s.project!.time_signatures);
  const points = useMemo(() => sortedTempoPoints({ tempo_points: tempoPoints }), [tempoPoints]);
  const signatures = useMemo(() => sortedSignatures({ time_signatures: timeSignatures }), [timeSignatures]);
  const tempo = useTempoMap();
  const view = useMemo(() => createTimelineViewStore({ pxPerBeat: 12 }), []);
  const bodyRef = useRef<HTMLDivElement>(null);
  useTimelineWheel(bodyRef, view);
  const [selection, setSelection] = useState<Selection>(null);

  const selTempo = selection?.kind === "tempo" ? points.find((p) => p.id === selection.id) ?? null : null;
  const selSig = selection?.kind === "signature" ? signatures.find((p) => p.id === selection.id) ?? null : null;
  const select = (kind: "tempo" | "signature") => (id: string | null) => setSelection(id ? { kind, id } : null);

  const playheadAt = () => Math.max(0, playheadStore.getPlayhead()?.transport.position ?? 0);
  const addTempoAtPlayhead = () => {
    const at = playheadAt();
    if (tempoTimeTaken(points, at)) return;
    void sendEdit(transport, addTempoPointCommand(at, tempo.bpmAt(at)));
  };
  const addSignatureAtPlayhead = () => {
    const sigAt = snapSignatureTime(signatures, playheadAt());
    if (sigAt === null) return;
    void sendEdit(transport, addSignatureCommand(sigAt, tempo.signatureAt(sigAt)));
  };
  const setSignature = (id: string, s: TimeSignature) => void sendEdit(transport, editSignatureCommand(id, { signature: s }));

  return (
    <div className="eth-tempo" data-feature="tempo" data-testid="tempo-editor">
      <div className="eth-tempo__row eth-tempo__row--ruler">
        <div className="eth-tempo__corner" style={{ width: HEADER_WIDTH }}>
          <Button size="sm" onClick={addTempoAtPlayhead} title="Add a tempo change at the playhead">
            + Tempo
          </Button>
          <Button size="sm" onClick={addSignatureAtPlayhead} title="Add a time signature change on the bar at the playhead">
            + Signature
          </Button>
        </div>
        <div className="eth-tempo__main">
          <Ruler view={view} showTempo={false} />
        </div>
      </div>
      <div className="eth-tempo__body" ref={bodyRef}>
        <div className="eth-tempo__row">
          <div className="eth-tempo__header" style={{ width: HEADER_WIDTH }}>
            <span className="eth-tempo__label">Signature</span>
            {selSig && (
              <span className="eth-tempo__fields" data-testid="signature-fields">
                <NumberField
                  size="sm"
                  aria-label="Beats per bar"
                  value={selSig.signature.numerator}
                  min={1}
                  max={99}
                  onChange={(n) => setSignature(selSig.id, { ...selSig.signature, numerator: n })}
                />
                <span className="eth-tempo__slash">/</span>
                <Select<string>
                  size="sm"
                  aria-label="Beat unit"
                  options={DENOMINATOR_OPTIONS}
                  value={String(selSig.signature.denominator)}
                  onChange={(d) => setSignature(selSig.id, { ...selSig.signature, denominator: Number(d) })}
                />
                {!isAtZero(selSig) && (
                  <Button
                    size="sm"
                    tone="ghost"
                    aria-label="Delete time signature"
                    title="Delete time signature"
                    onClick={() => {
                      setSelection(null);
                      void sendEdit(transport, removeSignaturesCommand([selSig.id]));
                    }}
                  >
                    ✕
                  </Button>
                )}
              </span>
            )}
          </div>
          <div className="eth-tempo__main">
            <SignatureLane
              view={view}
              tempo={tempo}
              signatures={signatures}
              selected={selSig?.id ?? null}
              onSelect={select("signature")}
            />
          </div>
        </div>
        <div className="eth-tempo__row eth-tempo__row--grow">
          <div className="eth-tempo__header" style={{ width: HEADER_WIDTH }}>
            <span className="eth-tempo__label">Tempo</span>
            {selTempo && (
              <span className="eth-tempo__fields eth-tempo__fields--column" data-testid="tempo-fields">
                <NumberField
                  size="sm"
                  aria-label="Tempo point BPM"
                  value={selTempo.bpm}
                  min={MIN_BPM}
                  max={MAX_BPM}
                  precision={2}
                  unit="BPM"
                  onChange={(bpm) => void sendEdit(transport, editTempoPointCommand(selTempo.id, { bpm }))}
                />
                <span className="eth-tempo__fields">
                  <Select<TempoCurve>
                    size="sm"
                    aria-label="Tempo curve"
                    options={CURVES}
                    value={selTempo.curve}
                    onChange={(curve) => void sendEdit(transport, editTempoPointCommand(selTempo.id, { curve }))}
                  />
                  {!isAtZero(selTempo) && (
                    <Button
                      size="sm"
                      tone="ghost"
                      aria-label="Delete tempo point"
                      title="Delete tempo point"
                      onClick={() => {
                        setSelection(null);
                        void sendEdit(transport, removeTempoPointsCommand([selTempo.id]));
                      }}
                    >
                      ✕
                    </Button>
                  )}
                </span>
              </span>
            )}
          </div>
          <div className="eth-tempo__main">
            <TempoLane view={view} tempo={tempo} points={points} selected={selTempo?.id ?? null} onSelect={select("tempo")} />
          </div>
        </div>
        <div className="eth-tempo__playhead" style={{ left: HEADER_WIDTH }}>
          <PlayheadLine view={view} />
        </div>
      </div>
    </div>
  );
}
