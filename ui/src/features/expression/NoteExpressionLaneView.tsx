/**
 * Per-note expression lane (`NoteExpressionKind`; `midi-expression` edits `Pressure`, the
 * `mpe` node adds `Pitch` / `Timbre` with the same component): each note's curve is drawn
 * over the note's span (times are beats from the note start).
 *
 * - Drag: pencil over the selected notes (all shown notes when none is selected). Every
 *   note under the stroke gets its curve's stroke range replaced (`SetNoteExpression` per
 *   note, batched per pointer move, one gesture = one undo step).
 * - Right-click: clear the curves of the selected (or all shown) notes.
 */

import { useMemo, useRef } from "react";
import type { Command, Note, NoteExpressionKind, NoteId } from "@/generated";
import { openContextMenu } from "@/kit";
import { beatsToPx, pxToBeats, useSelectedItems, type TimelineViewport } from "@/timeline";
import { cmd, newId, useTransport } from "@/transport";
import { startDrag } from "../piano-roll/drag";
import { curvePath, StrokeSampler, valueToY, yToValue } from "./curve";
import { useNoteExpressions } from "./hooks";
import { noteExpressionRange, noteKindLabel, replaceRange, strokePoints, type Range } from "./model";

export interface NoteExpressionLaneViewProps {
  kind: NoteExpressionKind;
  notes: ReadonlyArray<Note>;
  vp: TimelineViewport;
  widthPx: number;
  height: number;
  /**
   * Values shown, inside the kind's range (`mpe`: the Pitch lane shows the track's per-note
   * bend range, not the full ±96 semitones). Default: the kind's range.
   */
  range?: Range;
}

const PENCIL_PX = 3;

export function NoteExpressionLaneView({ kind, notes, vp, widthPx, height, range: shownRange }: NoteExpressionLaneViewProps) {
  const transport = useTransport();
  const svgRef = useRef<SVGSVGElement>(null);
  const range = shownRange ?? noteExpressionRange(kind);
  const label = noteKindLabel(kind);
  const selectedIds = useSelectedItems("note");
  const noteIds = useMemo(() => notes.map((n) => n.id), [notes]);
  const exprs = useNoteExpressions(noteIds, kind);
  const byNote = useMemo(() => new Map(exprs.map((e) => [e.note, e])), [exprs]);
  const targets = useMemo(() => {
    const sel = notes.filter((n) => selectedIds.has(n.id));
    return sel.length ? sel : notes;
  }, [notes, selectedIds]);

  const toY = (v: number) => valueToY(v, range, height);
  const width = Math.max(0, widthPx);

  const onPointerDown = (e: React.PointerEvent<SVGSVGElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const r = svgRef.current?.getBoundingClientRect();
    const x0 = e.clientX - (r?.left ?? 0);
    const y0 = e.clientY - (r?.top ?? 0);
    const sampler = new StrokeSampler(PENCIL_PX / vp.pxPerBeat);
    sampler.add(pxToBeats(x0, vp), yToValue(y0, range, height));
    // Curves as they were when the stroke started (each move re-applies the whole stroke).
    const before = new Map(targets.map((n) => [n.id, byNote.get(n.id)?.points ?? []]));
    const ids = new Map<NoteId, string>(targets.map((n) => [n.id, byNote.get(n.id)?.id ?? newId()]));
    startDrag(
      transport,
      e,
      {
        move: (dx, dy) => {
          sampler.add(pxToBeats(x0 + dx, vp), yToValue(y0 + dy, range, height));
          const s = strokePoints(sampler.samples(), range, sampler.minGap);
          if (!s) return null;
          const commands: Command[] = [];
          for (const n of targets) {
            const end = n.start + n.duration;
            if (s.end <= n.start || s.start >= end) continue;
            const from = Math.max(0, s.start - n.start);
            const to = s.end - n.start;
            const pts = s.points
              .filter((p) => p.time >= n.start && p.time < end)
              .map((p) => ({ ...p, time: p.time - n.start }));
            const next = replaceRange(before.get(n.id) ?? [], from, to, pts);
            if (typeof next === "string") continue;
            commands.push(cmd("Expression", { type: "SetNoteExpression", id: ids.get(n.id)!, note: n.id, kind, points: next }));
          }
          return commands.length ? cmd("Edit", { type: "Batch", label: `Draw ${label}`, commands }) : null;
        },
      },
      { threshold: 2, cursor: "crosshair" },
    );
  };

  const onContextMenu = (e: React.MouseEvent) => {
    const withCurve = targets.filter((n) => byNote.has(n.id)).map((n) => n.id);
    openContextMenu(e, [
      {
        label: `Clear ${label}`,
        disabled: withCurve.length === 0,
        onSelect: () => {
          transport
            .send(cmd("Expression", { type: "ClearNoteExpressions", notes: withCurve, kind }))
            .catch((err: unknown) => console.warn("[expression] command failed:", err));
        },
      },
    ]);
  };

  const t0 = pxToBeats(0, vp);
  const t1 = pxToBeats(width, vp);
  // Unknown width (not laid out yet): everything.
  const shown = width > 0 ? notes.filter((n) => n.start <= t1 && n.start + n.duration >= t0) : notes;

  return (
    <svg
      ref={svgRef}
      className="eth-expr-lane"
      width={width}
      height={height}
      data-testid="note-expression-lane"
      data-kind={label}
      aria-label={`${label} lane`}
      onPointerDown={onPointerDown}
      onContextMenu={onContextMenu}
    >
      {range[0] < 0 && range[1] > 0 && <line className="eth-expr-lane__centre" x1={0} x2={width} y1={toY(0)} y2={toY(0)} />}
      {shown.map((n) => {
        const xa = beatsToPx(n.start, vp);
        const xb = beatsToPx(n.start + n.duration, vp);
        const e = byNote.get(n.id);
        const d = e
          ? curvePath(
              e.points,
              (t) => beatsToPx(n.start + t, vp),
              (x) => pxToBeats(x, vp) - n.start,
              toY,
              xa,
              xb,
            )
          : "";
        return (
          <g key={n.id} data-note-id={n.id}>
            <rect
              className={selectedIds.has(n.id) ? "eth-expr-note eth-expr-note--selected" : "eth-expr-note"}
              x={xa}
              y={0}
              width={Math.max(1, xb - xa)}
              height={height}
            />
            {d && (
              <>
                <clipPath id={`eth-expr-clip-${n.id}`}>
                  <rect x={xa} y={0} width={Math.max(1, xb - xa)} height={height} />
                </clipPath>
                <path className="eth-expr-lane__curve" d={d} clipPath={`url(#eth-expr-clip-${n.id})`} data-testid="note-expression-curve" />
              </>
            )}
          </g>
        );
      })}
    </svg>
  );
}
