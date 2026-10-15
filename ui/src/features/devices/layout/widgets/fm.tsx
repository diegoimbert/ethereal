/**
 * FmAlgorithm (fm-synth): the algorithm picker of an FM synth. One small routing diagram
 * per algorithm (`fmDiagram`), the selected one highlighted; click or Enter selects it.
 * Arrow keys move through the algorithms like a radio group.
 */

import clsx from "clsx";
import { useRef, type KeyboardEvent } from "react";
import type { FmRouting } from "@/generated";
import { labelIndex } from "@/domain/paramScale";
import { useParam } from "../context";
import { fmDiagram } from "../fmDiagram";
import { Missing, TypedFrame, type TypedProps } from "./typed";
import { useT } from "@/i18n";

/** Diagram box in SVG units (scaled to the cell). */
const W = 60;
const H = 64;
const BOX_W = 14;
const BOX_H = 11;

function Diagram({ routing }: { routing: FmRouting }) {
  const d = fmDiagram(routing);
  const at = (op: number) => d.nodes.find((n) => n.op === op)!;
  const px = (x: number) => x * W;
  const py = (y: number) => y * H;
  const carriers = d.nodes.filter((n) => n.carrier);
  const outY = py(d.outputY);
  const outX0 = Math.min(...carriers.map((n) => px(n.x))) - BOX_W / 2;
  const outX1 = Math.max(...carriers.map((n) => px(n.x))) + BOX_W / 2;
  return (
    <svg className="eth-fm__svg" viewBox={`0 0 ${W} ${H}`} aria-hidden="true">
      {d.edges.map(({ from, to }) => {
        const a = at(from);
        const b = at(to);
        return <line key={`${from}-${to}`} className="eth-fm__edge" x1={px(a.x)} y1={py(a.y) + BOX_H / 2} x2={px(b.x)} y2={py(b.y) - BOX_H / 2} />;
      })}
      {carriers.map((n) => (
        <line key={`out-${n.op}`} className="eth-fm__edge" x1={px(n.x)} y1={py(n.y) + BOX_H / 2} x2={px(n.x)} y2={outY} />
      ))}
      <line className="eth-fm__out" x1={outX0} y1={outY} x2={outX1} y2={outY} />
      {d.nodes.map((n) => (
        <g key={n.op} className={clsx("eth-fm__op", n.carrier && "eth-fm__op--carrier")}>
          <rect x={px(n.x) - BOX_W / 2} y={py(n.y) - BOX_H / 2} width={BOX_W} height={BOX_H} rx={2} />
          <text x={px(n.x)} y={py(n.y)} dominantBaseline="central" textAnchor="middle">
            {n.name}
          </text>
        </g>
      ))}
    </svg>
  );
}

export function FmAlgorithmWidget({ widget: w, size, label }: TypedProps<"FmAlgorithm">) {
  const t = useT();
  const algorithm = useParam(w.algorithm);
  const cells = useRef<Array<HTMLButtonElement | null>>([]);
  if (!algorithm) return <Missing type="FmAlgorithm" />;
  const current = labelIndex(algorithm.info, algorithm.plain);
  const labels = algorithm.info.labels ?? [];
  const select = (i: number) => algorithm.setPlain(algorithm.info.min + i);
  const onKey = (e: KeyboardEvent, i: number) => {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const next = (i + step + w.algorithms.length) % w.algorithms.length;
    select(next);
    cells.current[next]?.focus();
  };
  return (
    <TypedFrame type="fm-algorithm" size={size} label={label}>
      <div className="eth-fm" role="radiogroup" aria-label={`${algorithm.info.name}: ${algorithm.text}`} data-testid="widget-fm-algorithm">
        {w.algorithms.map((routing, i) => (
          <button
            key={i}
            ref={(el) => {
              cells.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={i === current}
            tabIndex={i === current ? 0 : -1}
            className={clsx("eth-fm__cell", i === current && "eth-fm__cell--on")}
            title={`${i + 1}: ${labels[i] ?? ""}`}
            aria-label={t("devices.widget.fm_algorithm", { n: i + 1, name: labels[i] ?? "" })}
            onClick={() => select(i)}
            onKeyDown={(e) => onKey(e, i)}
          >
            <span className="eth-fm__num">{i + 1}</span>
            <Diagram routing={routing} />
          </button>
        ))}
      </div>
      <span className="eth-fm__caption">{labels[current] ?? algorithm.text}</span>
    </TypedFrame>
  );
}
