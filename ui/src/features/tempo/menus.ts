/**
 * Context menus and data hooks shared by the tempo lanes and the ruler markers.
 */

import { useMemo } from "react";
import type { Beats, TempoPoint, TimeSignature, TimeSignaturePoint } from "@/generated";
import type { GridStep } from "@/timeline/grid";
import type { ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state/projectStore";
import type { EngineTransport } from "@/transport";
import type { TempoMap } from "@/timeline/tempoMap";
import { sendEdit } from "./gesture";
import {
  addSignatureCommand,
  addTempoPointCommand,
  formatSignature,
  isAtZero,
  snapSignatureTime,
  sortedSignatures,
  sortedTempoPoints,
  tempoTimeTaken,
} from "./tempoCommands";

export const COMMON_SIGNATURES: ReadonlyArray<TimeSignature> = [
  { numerator: 2, denominator: 4 },
  { numerator: 3, denominator: 4 },
  { numerator: 4, denominator: 4 },
  { numerator: 5, denominator: 4 },
  { numerator: 6, denominator: 8 },
  { numerator: 7, denominator: 8 },
  { numerator: 12, denominator: 8 },
];

/** Context-menu entries for a signature change (shared with the ruler markers). */
export function signatureMenu(
  point: TimeSignaturePoint,
  set: (s: TimeSignature) => void,
  remove: () => void,
): ContextMenuEntry[] {
  const same = (s: TimeSignature) => s.numerator === point.signature.numerator && s.denominator === point.signature.denominator;
  return [
    ...COMMON_SIGNATURES.map((s) => ({ label: formatSignature(s), disabled: same(s), onSelect: () => set(s) })),
    "separator",
    { label: "Delete Time Signature", shortcut: "⌫", danger: true, disabled: isAtZero(point), onSelect: remove },
  ];
}

const EMPTY_TEMPO: Record<string, TempoPoint> = {};
const EMPTY_SIGS: Record<string, TimeSignaturePoint> = {};

/**
 * Ruler context-menu entries at `beats` (snapped to the ruler's grid): add a tempo change /
 * a time signature there. The signature change snaps `sig.raw` (default `beats`) to at
 * least a beat of the signature in effect (`snapSignatureTime`), so it may sit mid-bar.
 */
export function rulerTempoMenu(
  transport: EngineTransport,
  tempo: TempoMap,
  points: ReadonlyArray<TempoPoint>,
  signatures: ReadonlyArray<TimeSignaturePoint>,
  beats: Beats,
  sig: { raw?: Beats; step?: GridStep | null; free?: boolean } = {},
): ContextMenuEntry[] {
  const at = Math.max(0, beats);
  const sigAt = snapSignatureTime(signatures, sig.raw ?? at, { step: sig.step, free: sig.free });
  return [
    {
      label: "Add Tempo Change Here",
      disabled: tempoTimeTaken(points, at),
      onSelect: () => void sendEdit(transport, addTempoPointCommand(at, tempo.bpmAt(at))),
    },
    {
      label: "Add Time Signature Change Here",
      disabled: sigAt === null,
      onSelect: () => {
        if (sigAt !== null) void sendEdit(transport, addSignatureCommand(sigAt, tempo.signatureAt(sigAt)));
      },
    },
  ];
}

/** The project's tempo points and signatures, sorted (for the ruler menu). */
export function useSortedTempoMap(): { points: TempoPoint[]; signatures: TimeSignaturePoint[] } {
  const tp = useProjectStore((s) => s.project?.tempo_points ?? EMPTY_TEMPO);
  const ts = useProjectStore((s) => s.project?.time_signatures ?? EMPTY_SIGS);
  return useMemo(
    () => ({ points: sortedTempoPoints({ tempo_points: tp }), signatures: sortedSignatures({ time_signatures: ts }) }),
    [tp, ts],
  );
}
