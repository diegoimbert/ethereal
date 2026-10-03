/**
 * The multisampler's zone editor (`Widget::ZoneMap`): a key × velocity map of the zones
 * (`BuiltinDevice::MultiSampler::zones`) with a keyboard strip, and an inspector for the
 * selected zone.
 *
 * - Click a zone to select it; drag its body to move it, its edges to resize its key or
 *   velocity range. The map previews the drag and sends one `Device::SetZones` on release
 *   (one undo step).
 * - Drop samples from the Browser (one) or from the computer (several) onto a key: they
 *   become zones mapped across the keys (`mapDropped`: roots from names like `C3` / `60`).
 *   Import + zones share one gesture for a Browser drop.
 * - Delete / Backspace removes the selected zone.
 */

import clsx from "clsx";
import { useRef, useState, type DragEvent, type KeyboardEvent } from "react";
import type { GestureId, SampleZone, WidgetSize } from "@/generated";
import { hasBrowserDrag, readBrowserDrag, resolveDroppedMedia } from "@/features/browser/dragPayload";
import { droppedFiles, hasOsFiles, importAudio, noteHover, type ImportSource } from "@/features/import";
import { useProjectStore } from "@/state";
import { cmd, nextGestureId, useTransport } from "@/transport";
import { useLayoutContext } from "../context";
import { Plot } from "../plot";
import { TypedFrame } from "../widgets/typed";
import { ZoneInspector } from "./ZoneInspector";
import {
  KEYS,
  dragZone,
  hitTest,
  keyToX,
  mapDropped,
  noteName,
  withZone,
  xToKey,
  zoneRect,
  type DroppedSample,
  type Edge,
  type MapSize,
} from "./zoneMath";
import "./zones.css";

/** Edge grab tolerance in px (pointer tolerance, not a visual size). */
const HIT = 6;
const BLACK = new Set([1, 3, 6, 8, 10]);

export interface ZoneMapWidgetProps {
  widget: { type: "ZoneMap" };
  size: WidgetSize;
  label: string | null;
}

export function ZoneMapWidget({ size, label }: ZoneMapWidgetProps) {
  const { device, sender } = useLayoutContext();
  const transport = useTransport();
  const kind = device.kind;
  const zones: ReadonlyArray<SampleZone> = kind.type === "Builtin" && kind.device.type === "MultiSampler" ? kind.device.zones : [];
  const [selected, setSelected] = useState<number | null>(null);
  const [draft, setDraft] = useState<SampleZone[] | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const plotSize = useRef<MapSize>({ w: 1, h: 1 });
  const grab = useRef<{ zone: number; part: Edge | "body"; x: number; y: number; start: SampleZone; moved: SampleZone[] | null } | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const shown = draft ?? zones;
  const sel = selected !== null && selected < zones.length ? selected : null;

  const commit = (next: ReadonlyArray<SampleZone>) =>
    sender.send(cmd("Device", { type: "SetZones", device: device.id, zones: [...next] }));

  const keyAt = (clientX: number) => {
    const r = box.current?.getBoundingClientRect();
    if (!r || r.width <= 0 || !Number.isFinite(clientX)) return 60;
    return xToKey(clientX - r.left, { w: r.width, h: r.height });
  };

  const addSamples = async (samples: DroppedSample[], key: number, opts: { gesture?: GestureId } = {}) => {
    const added = mapDropped(samples, key, zones.length === 0);
    if (added.length === 0) return;
    const base = currentZones(device.id) ?? zones;
    await transport.send(cmd("Device", { type: "SetZones", device: device.id, zones: [...base, ...added] }), opts);
    setSelected(base.length + added.length - 1);
  };

  const importFiles = async (sources: ImportSource[], key: number) => {
    const outcomes = await importAudio(transport, sources);
    const ok = outcomes.flatMap((o) => (o.media ? [{ media: o.media.id, name: o.media.name }] : []));
    const failed = outcomes.find((o) => o.error && o.error !== "cancelled");
    if (failed) setError(failed.error);
    await addSamples(ok, key);
  };

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    const browser = hasBrowserDrag(e.dataTransfer);
    const files = hasOsFiles(e.dataTransfer);
    if (!browser && !files) return;
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = "copy";
    const key = keyAt(e.clientX);
    setOver(key);
    // Desktop path drops arrive through the hover registry.
    if (files) noteHover(e, (sources) => void importFiles(sources, key));
  };

  const onDrop = async (e: DragEvent<HTMLDivElement>) => {
    setOver(null);
    const key = keyAt(e.clientX);
    setError(null);
    if (hasOsFiles(e.dataTransfer)) {
      e.preventDefault();
      e.stopPropagation();
      await importFiles(droppedFiles(e.dataTransfer), key).catch((err: unknown) => setError(String(err)));
      return;
    }
    const payload = readBrowserDrag(e.dataTransfer);
    if (!payload) return;
    e.preventDefault();
    e.stopPropagation();
    const gesture = nextGestureId();
    try {
      const m = await resolveDroppedMedia(transport, payload, { gesture });
      await addSamples([{ media: m.id, name: m.name }], key, { gesture });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (sel === null) return;
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      void commit(zones.filter((_, i) => i !== sel));
      setSelected(null);
    } else if (e.key === "Escape") {
      setSelected(null);
    }
  };

  const describe = shown.length === 0 ? "No zones" : `${shown.length} zone${shown.length === 1 ? "" : "s"}`;

  return (
    <TypedFrame type="zone-map" size={size} label={label}>
      <div
        ref={box}
        className={clsx("eth-zones", over !== null && "eth-zones--over")}
        data-testid="zone-map"
        tabIndex={0}
        aria-label="Zone map"
        onKeyDown={onKeyDown}
        onDragOver={onDragOver}
        onDragLeave={() => setOver(null)}
        onDrop={(e) => void onDrop(e)}
      >
        <Plot
          className="eth-plot--zones"
          label={describe}
          testId="widget-zone-map"
          drag={{
            start: (x, y) => {
              const s = plotSize.current;
              const g = hitTest(shown, sel, x, y, s, HIT);
              if (!g) {
                setSelected(null);
                return false;
              }
              setSelected(g.zone);
              grab.current = { ...g, x, y, start: shown[g.zone]!, moved: null };
              box.current?.focus({ preventScroll: true });
            },
            move: (x, y) => {
              const g = grab.current;
              if (!g) return;
              const s = plotSize.current;
              const dKey = Math.round(((x - g.x) / Math.max(1, s.w)) * KEYS);
              const dVel = Math.round(((g.y - y) / Math.max(1, s.h)) * 127);
              const z = dragZone(g.start, g.part, dKey, dVel);
              const next = withZone(zones, g.zone, z);
              g.moved = next;
              setDraft(next);
            },
            end: () => {
              const g = grab.current;
              grab.current = null;
              setDraft(null);
              if (g?.moved && JSON.stringify(g.moved[g.zone]) !== JSON.stringify(g.start)) void commit(g.moved);
            },
          }}
        >
          {(s) => {
            plotSize.current = s;
            return <ZoneLayer zones={shown} selected={sel} size={s} over={over} />;
          }}
        </Plot>
        <Keyboard selected={sel !== null ? (shown[sel] ?? null) : null} over={over} />
      </div>
      {error && (
        <span className="eth-zones__error" role="alert">
          {error}
        </span>
      )}
      {sel !== null && zones[sel] ? (
        <ZoneInspector
          zone={zones[sel]!}
          index={sel}
          onChange={(z) => void commit(withZone(zones, sel, z))}
          onRemove={() => {
            void commit(zones.filter((_, i) => i !== sel));
            setSelected(null);
          }}
        />
      ) : (
        <span className="eth-zones__hint">
          {zones.length === 0 ? "Drop samples here from the Browser or your computer" : "Select a zone to edit it"}
        </span>
      )}
    </TypedFrame>
  );
}

/** The zones of `device` in the store right now (after an import landed). */
function currentZones(device: string): ReadonlyArray<SampleZone> | null {
  const d = useProjectStore.getState().project?.devices[device];
  return d && d.kind.type === "Builtin" && d.kind.device.type === "MultiSampler" ? d.kind.device.zones : null;
}

function ZoneLayer({ zones, selected, size, over }: { zones: ReadonlyArray<SampleZone>; selected: number | null; size: MapSize; over: number | null }) {
  const octaves = Array.from({ length: 10 }, (_, o) => (o + 1) * 12);
  return (
    <>
      <g className="eth-zones__grid">
        {octaves.map((k) => (
          <line key={k} x1={keyToX(k, size)} x2={keyToX(k, size)} y1={0} y2={size.h} />
        ))}
      </g>
      {zones.map((z, i) => {
        const r = zoneRect(z, size);
        return (
          <g key={i} className={clsx("eth-zones__zone", i === selected && "eth-zones__zone--selected", !z.media && "eth-zones__zone--empty")} data-zone={i}>
            <rect x={r.x} y={r.y} width={r.w} height={r.h}>
              <title>{`Zone ${i + 1}: ${noteName(z.keys.lo)}–${noteName(z.keys.hi)}, velocity ${z.velocities.lo}–${z.velocities.hi}, root ${noteName(z.root_key)}${
                z.round_robin ? `, round robin ${z.round_robin}` : ""
              }`}</title>
            </rect>
            {i === selected && (
              <>
                <line className="eth-zones__edge eth-zones__edge--x" x1={r.x} x2={r.x} y1={r.y} y2={r.y + r.h} />
                <line className="eth-zones__edge eth-zones__edge--x" x1={r.x + r.w} x2={r.x + r.w} y1={r.y} y2={r.y + r.h} />
                <line className="eth-zones__edge eth-zones__edge--y" x1={r.x} x2={r.x + r.w} y1={r.y} y2={r.y} />
                <line className="eth-zones__edge eth-zones__edge--y" x1={r.x} x2={r.x + r.w} y1={r.y + r.h} y2={r.y + r.h} />
              </>
            )}
            <line className="eth-zones__root" x1={keyToX(z.root_key + 0.5, size)} x2={keyToX(z.root_key + 0.5, size)} y1={r.y} y2={r.y + r.h} />
          </g>
        );
      })}
      {over !== null && <rect className="eth-zones__drop" x={keyToX(over, size)} y={0} width={Math.max(1, keyToX(1, size))} height={size.h} />}
      {zones.length === 0 && over === null && (
        <text className="eth-plot__empty" x={size.w / 2} y={size.h / 2}>
          No zones
        </text>
      )}
    </>
  );
}

/** Keyboard strip under the map: octave labels, the selected zone's range and root. */
function Keyboard({ selected, over }: { selected: SampleZone | null; over: number | null }) {
  return (
    <Plot className="eth-zones__keys" label="Keyboard">
      {(s) => (
        <>
          {Array.from({ length: KEYS }, (_, k) => {
            const inRange = selected !== null && k >= selected.keys.lo && k <= selected.keys.hi;
            return (
              <rect
                key={k}
                className={clsx(
                  "eth-zones__key",
                  BLACK.has(k % 12) && "eth-zones__key--black",
                  inRange && "eth-zones__key--range",
                  selected?.root_key === k && "eth-zones__key--root",
                  over === k && "eth-zones__key--over",
                )}
                x={keyToX(k, s)}
                y={0}
                width={Math.max(1, keyToX(1, s))}
                height={s.h}
              />
            );
          })}
          {Array.from({ length: 11 }, (_, o) => (
            <text key={o} className="eth-zones__octave" x={keyToX(o * 12, s) + 1} y={s.h / 2}>
              {noteName(o * 12)}
            </text>
          ))}
        </>
      )}
    </Plot>
  );
}
