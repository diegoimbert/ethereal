/**
 * The modulation decoration of every param widget (the `ParamModulation` seam of the shared
 * renderer, `features/devices/layout/seams.ts`):
 *
 * - **Depth ring** (knobs): one arc per mapping from the base to `base + depth` (the
 *   source's colour), and a dot at the live effective value (engine readback) while it
 *   plays. Other widgets get a thin range bar.
 * - **Depth chips**: one pill per mapping under the control: drag vertically to set the
 *   depth (one undo step per drag), double-click for 0, right-click to invert or remove.
 * - **Drop target**: while a source is being mapped (a modulator's map button or drag
 *   handle, a macro's handle), every param it may reach is highlighted: click or drop to
 *   map it (`Modulation::Map`, depth [`DEFAULT_DEPTH`]).
 * - **Macros** (a rack's params 0..8) are sources: they get a drag/map handle instead.
 */

import clsx from "clsx";
import type { DragEvent } from "react";
import { useMemo } from "react";
import type { Device, ModMapping, ModSource, ParamInfo } from "@/generated";
import { knobGeometry } from "@/theme/tokens";
import { useGestureSender, useSend } from "@/features/devices/gesture";
import { openContextMenu, useVerticalDrag } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, newId } from "@/transport";
import {
  DEFAULT_DEPTH,
  MACROS,
  MOD_DRAG_TYPE,
  canTarget,
  isChainRack,
  mappingsOf,
  sourceName,
  sourceSlot,
  useModMapping,
} from "./model";
import { useModulatedValue } from "./readback";
import "./modulation.css";

export interface ParamModulationProps {
  device: Device;
  info: ParamInfo;
  /** The base value (document), normalized 0..1. */
  normalized: number;
}

const START = Number(knobGeometry.startAngle);
const SWEEP = Number(knobGeometry.sweep);
/** Ring radius (viewBox units): just outside the knob's own arc. */
const RING_R = 48;

function polar(r: number, deg: number): [number, number] {
  const rad = ((deg - 90) * Math.PI) / 180;
  return [50 + r * Math.cos(rad), 50 + r * Math.sin(rad)];
}

function arc(from: number, to: number): string {
  const [a, b] = from <= to ? [from, to] : [to, from];
  const [x1, y1] = polar(RING_R, START + a * SWEEP);
  const [x2, y2] = polar(RING_R, START + b * SWEEP);
  const large = (b - a) * SWEEP > 180 ? 1 : 0;
  return `M ${x1} ${y1} A ${RING_R} ${RING_R} 0 ${large} 1 ${x2} ${y2}`;
}

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

export function ParamModulation({ device, info, normalized }: ParamModulationProps) {
  const project = useProjectStore((s) => s.project);
  const table = project?.mod_mappings;
  const mappings = useMemo(() => mappingsOf(table, device.id, info.id), [table, device.id, info.id]);
  const mapping = useModMapping((s) => s.source);
  // Pseudo devices (modulator panels) and unknown devices: nothing to decorate.
  const real = project?.devices[device.id];
  const live = useModulatedValue(device.id, info.id, !!real && mappings.length > 0);
  if (!project || !real) return <span className="eth-param__mod" aria-hidden="true" />;

  const isMacro = isChainRack(real) && info.id < MACROS;
  const target = mapping !== null && canTarget(project, mapping, real, info.id);
  return (
    <>
      <span className="eth-param__mod" aria-hidden="true" />
      {mappings.length > 0 && (
        <>
          <svg className="eth-mod-ring" viewBox="0 0 100 100" aria-hidden="true" data-testid="mod-ring">
            {mappings.map((m) => (
              <path
                key={m.id}
                className={clsx("eth-mod-ring__range", `eth-mod--slot-${sourceSlot(project, m.source)}`)}
                d={arc(normalized, clamp01(normalized + m.depth))}
              />
            ))}
            {live !== null && (
              <circle className="eth-mod-ring__live" cx={polar(RING_R, START + clamp01(live) * SWEEP)[0]} cy={polar(RING_R, START + clamp01(live) * SWEEP)[1]} r={5} />
            )}
          </svg>
          <span className="eth-mod-bar" aria-hidden="true">
            {mappings.map((m) => {
              const lo = Math.min(normalized, clamp01(normalized + m.depth));
              const hi = Math.max(normalized, clamp01(normalized + m.depth));
              return (
                <span
                  key={m.id}
                  className={clsx("eth-mod-bar__range", `eth-mod--slot-${sourceSlot(project, m.source)}`)}
                  style={{ left: `${lo * 100}%`, width: `${(hi - lo) * 100}%` }}
                />
              );
            })}
          </span>
          <span className="eth-mod-chips">
            {mappings.map((m) => (
              <DepthChip key={m.id} mapping={m} name={sourceName(project, m.source)} slot={sourceSlot(project, m.source)} param={info.name} />
            ))}
          </span>
        </>
      )}
      {isMacro && <MacroHandle rack={real.id} index={info.id} name={info.name} />}
      {target && mapping && <DropTarget source={mapping} device={real} info={info} />}
    </>
  );
}

/** Drag vertically to set a mapping's depth (-1..1). */
function DepthChip({ mapping: m, name, slot, param }: { mapping: ModMapping; name: string; slot: number; param: string }) {
  const sender = useGestureSender();
  const send = useSend();
  const setDepth = (depth: number) => {
    const d = Math.round(Math.max(-1, Math.min(1, depth)) * 1000) / 1000;
    if (d !== m.depth) void sender.send(cmd("Modulation", { type: "SetDepth", id: m.id, depth: d }));
  };
  const drag = useVerticalDrag({
    value: (m.depth + 1) / 2,
    onChange: (v) => setDepth(v * 2 - 1),
    sensitivity: 1 / 200,
    defaultValue: 0.5,
    onChangeStart: sender.begin,
    onChangeEnd: sender.end,
  });
  const pct = Math.round(m.depth * 100);
  return (
    <span
      className={clsx("eth-mod-chip", `eth-mod--slot-${slot}`)}
      role="slider"
      tabIndex={0}
      aria-label={`${name} depth on ${param}`}
      aria-valuemin={-1}
      aria-valuemax={1}
      aria-valuenow={m.depth}
      aria-valuetext={`${pct > 0 ? "+" : ""}${pct} %`}
      title={`${name}: ${pct > 0 ? "+" : ""}${pct} % (drag to set depth, double-click for 0)`}
      data-testid="mod-depth"
      {...drag}
      onContextMenu={(e) =>
        openContextMenu(e, [
          { label: "Invert Depth", onSelect: () => void send(cmd("Modulation", { type: "SetDepth", id: m.id, depth: -m.depth })) },
          "separator",
          { label: `Remove ${name} Modulation`, danger: true, onSelect: () => void send(cmd("Modulation", { type: "Unmap", id: m.id })) },
        ])
      }
    >
      <span className="eth-mod-chip__fill" style={{ left: `${50 + Math.min(0, m.depth) * 50}%`, width: `${Math.abs(m.depth) * 50}%` }} />
    </span>
  );
}

/** The highlighted drop target over a param while a source is being mapped. */
function DropTarget({ source, device, info }: { source: ModSource; device: Device; info: ParamInfo }) {
  const send = useSend();
  const stop = useModMapping((s) => s.stop);
  const map = (keep: boolean) => {
    void send(cmd("Modulation", { type: "Map", id: newId(), source, device: device.id, param: info.id, depth: DEFAULT_DEPTH }));
    if (!keep) stop();
  };
  const accepts = (e: DragEvent) => e.dataTransfer.types.includes(MOD_DRAG_TYPE);
  return (
    <button
      type="button"
      className="eth-mod-target"
      aria-label={`Modulate ${info.name}`}
      data-testid="mod-target"
      onClick={(e) => {
        e.stopPropagation();
        map(e.shiftKey);
      }}
      onDragOver={(e) => {
        if (!accepts(e)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "link";
      }}
      onDrop={(e) => {
        if (!accepts(e)) return;
        e.preventDefault();
        map(false);
      }}
    />
  );
}

/** A rack macro as a modulation source: drag onto a param, or click then click a param. */
function MacroHandle({ rack, index, name }: { rack: string; index: number; name: string }) {
  const source: ModSource = { type: "Macro", rack, index };
  const active = useModMapping((s) => s.source?.type === "Macro" && s.source.rack === rack && s.source.index === index);
  const { start, stop } = useModMapping.getState();
  return (
    <button
      type="button"
      className={clsx("eth-mod-handle", active && "eth-mod-handle--active")}
      aria-label={`Map ${name}`}
      aria-pressed={active}
      title={`Map ${name}: drag onto a parameter of a device in this rack, or click then click the parameter`}
      draggable
      onClick={(e) => {
        e.stopPropagation();
        if (active) stop();
        else start(source);
      }}
      onDragStart={(e) => {
        e.stopPropagation();
        e.dataTransfer.setData(MOD_DRAG_TYPE, JSON.stringify(source));
        e.dataTransfer.effectAllowed = "link";
        start(source);
      }}
      onDragEnd={() => stop()}
      onPointerDown={(e) => e.stopPropagation()}
    />
  );
}
