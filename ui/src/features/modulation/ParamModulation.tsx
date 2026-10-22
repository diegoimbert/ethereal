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
import { useT } from "@/i18n";
import { clamp01, openContextMenu, useVerticalDrag } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, newId } from "@/transport";
import {
  DEFAULT_DEPTH,
  DEVICE_ENABLED_PARAM,
  MACROS,
  MOD_DRAG_TYPE,
  canTarget,
  isChainRack,
  mappingsOf,
  sourceName,
  sourceSlot,
  switchDepth,
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
const RING_R = 52;

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

export function ParamModulation({ device, info, normalized }: ParamModulationProps) {
  const t = useT();
  // Narrow subscriptions: this renders inside every param widget, so it must not re-render
  // them all on every patch (only when this param's mappings or device change).
  const table = useProjectStore((s) => s.project?.mod_mappings);
  const real = useProjectStore((s) => s.project?.devices[device.id]);
  const mappings = useMemo(() => mappingsOf(table, device.id, info.id), [table, device.id, info.id]);
  const mapping = useModMapping((s) => s.source);
  const target = useProjectStore((s) => mapping !== null && !!real && !!s.project && canTarget(s.project, mapping, real, info.id));
  // Pseudo devices (modulator panels) and unknown devices: nothing to decorate.
  const live = useModulatedValue(device.id, info.id, !!real && mappings.length > 0);
  if (!real) return <span className="eth-param__mod" aria-hidden="true" />;

  const isMacro = isChainRack(real) && info.id < MACROS;
  return (
    <>
      <span className="eth-param__mod" aria-hidden="true" />
      {mappings.length > 0 && <Rings mappings={mappings} normalized={normalized} live={live} param={info.name} />}
      {isMacro && <MacroHandle rack={real.id} index={info.id} name={info.name} />}
      {target && mapping && <DropTarget source={mapping} device={real} param={info.id} label={t("modulation.target.modulate", { name: info.name })} />}
    </>
  );
}

/** Depth ring, range bar and depth chips of a modulated param. */
function Rings({ mappings, normalized, live, param }: { mappings: ModMapping[]; normalized: number; live: number | null; param: string }) {
  const project = useProjectStore((s) => s.project);
  if (!project) return null;
  return (
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
          <circle
            className="eth-mod-ring__live"
            cx={polar(RING_R, START + clamp01(live) * SWEEP)[0]}
            cy={polar(RING_R, START + clamp01(live) * SWEEP)[1]}
            r={5}
          />
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
          <DepthChip key={m.id} mapping={m} name={sourceName(project, m.source)} slot={sourceSlot(project, m.source)} param={param} />
        ))}
      </span>
    </>
  );
}

/** Drag vertically to set a mapping's depth (-1..1). */
function DepthChip({ mapping: m, name, slot, param }: { mapping: ModMapping; name: string; slot: number; param: string }) {
  const t = useT();
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
      aria-label={t("modulation.chip.label", { name, param })}
      aria-valuemin={-1}
      aria-valuemax={1}
      aria-valuenow={m.depth}
      aria-valuetext={`${pct > 0 ? "+" : ""}${pct} %`}
      title={t("modulation.chip.title", { name, depth: `${pct > 0 ? "+" : ""}${pct}` })}
      data-testid="mod-depth"
      {...drag}
      onContextMenu={(e) =>
        openContextMenu(e, [
          { label: t("modulation.chip.invert"), onSelect: () => void send(cmd("Modulation", { type: "SetDepth", id: m.id, depth: -m.depth })) },
          "separator",
          { label: t("modulation.chip.remove", { name }), danger: true, onSelect: () => void send(cmd("Modulation", { type: "Unmap", id: m.id })) },
        ])
      }
    >
      <span className="eth-mod-chip__fill" style={{ left: `${50 + Math.min(0, m.depth) * 50}%`, width: `${Math.abs(m.depth) * 50}%` }} />
    </span>
  );
}

/**
 * The modulation decoration of a device's on/off button (`DEVICE_ENABLED_PARAM`): a dot per
 * source switching it (its colour; right-click to invert or remove), and the drop target
 * while a source that may switch it is being mapped. The button itself shows the switch's
 * live state (`useDeviceOn`).
 */
export function SwitchModulation({ device }: { device: Device }) {
  const t = useT();
  const send = useSend();
  const table = useProjectStore((s) => s.project?.mod_mappings);
  const project = useProjectStore((s) => s.project);
  const mappings = useMemo(() => mappingsOf(table, device.id, DEVICE_ENABLED_PARAM), [table, device.id]);
  const mapping = useModMapping((s) => s.source);
  const target = useProjectStore((s) => mapping !== null && !!s.project && canTarget(s.project, mapping, device, DEVICE_ENABLED_PARAM));
  if (!project) return null;
  return (
    <>
      {mappings.length > 0 && (
        <span className="eth-mod-switch" data-testid="mod-switch">
          {mappings.map((m) => {
            const name = sourceName(project, m.source);
            return (
              <span
                key={m.id}
                className={clsx("eth-mod-switch__dot", `eth-mod--slot-${sourceSlot(project, m.source)}`)}
                role="img"
                aria-label={t("modulation.switch.dot", { name })}
                title={t("modulation.switch.dot", { name })}
                onContextMenu={(e) =>
                  openContextMenu(e, [
                    { label: t("modulation.chip.invert"), onSelect: () => void send(cmd("Modulation", { type: "SetDepth", id: m.id, depth: -m.depth })) },
                    "separator",
                    { label: t("modulation.chip.remove", { name }), danger: true, onSelect: () => void send(cmd("Modulation", { type: "Unmap", id: m.id })) },
                  ])
                }
              />
            );
          })}
        </span>
      )}
      {target && mapping && (
        <DropTarget source={mapping} device={device} param={DEVICE_ENABLED_PARAM} label={t("modulation.target.switch", { name: device.name })} />
      )}
    </>
  );
}

/** The highlighted drop target over a param (or a device's switch) while a source is being mapped. */
function DropTarget({ source, device, param, label }: { source: ModSource; device: Device; param: number; label: string }) {
  const send = useSend();
  const stop = useModMapping((s) => s.stop);
  const depth = param === DEVICE_ENABLED_PARAM ? switchDepth(device) : DEFAULT_DEPTH;
  const map = (keep: boolean) => {
    void send(cmd("Modulation", { type: "Map", id: newId(), source, device: device.id, param, depth }));
    if (!keep) stop();
  };
  const accepts = (e: DragEvent) => e.dataTransfer.types.includes(MOD_DRAG_TYPE);
  return (
    <button
      type="button"
      className="eth-mod-target"
      aria-label={label}
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
  const t = useT();
  const source: ModSource = { type: "Macro", rack, index };
  const active = useModMapping((s) => s.source?.type === "Macro" && s.source.rack === rack && s.source.index === index);
  const { start, stop } = useModMapping.getState();
  return (
    <button
      type="button"
      className={clsx("eth-mod-handle", active && "eth-mod-handle--active")}
      aria-label={t("modulation.card.map", { name })}
      aria-pressed={active}
      title={t("modulation.macro.map_title", { name })}
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
        window.setTimeout(() => start(source), 0);
      }}
      onDragEnd={() => stop()}
      onPointerDown={(e) => e.stopPropagation()}
    />
  );
}
