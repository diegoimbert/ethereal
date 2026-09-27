import clsx from "clsx";
import { useEffect, useRef, useState, type PointerEvent } from "react";
import type { AutoSlice, Device, MediaRef, PeakData } from "@/generated";
import { useSend } from "@/features/devices/gesture";
import { SampleSlot } from "@/features/devices/SampleSlot";
import { Button, NumberField, openContextMenu, readToken, Select, Toggle, useTheme } from "@/kit";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, newId, useTransport } from "@/transport";
import { noteName, playableSlices, sampleOf, slicePadIds, slicesOf } from "./padUtils";

/** Peaks fetched for the overview (one request per media). */
const OVERVIEW_PEAKS = 1024;

type AutoMode = AutoSlice["type"];
const AUTO_MODES: { value: AutoMode; label: string }[] = [
  { value: "Transients", label: "Transients" },
  { value: "Grid", label: "Beat grid" },
  { value: "Equal", label: "Equal" },
];
const GRID_OPTIONS = [
  { value: "0.25", label: "1/16" },
  { value: "0.5", label: "1/8" },
  { value: "1", label: "1/4" },
  { value: "2", label: "1/2" },
  { value: "4", label: "1 bar" },
];

/** Whole-sample min/max peaks of `media`, refetched when the engine reports new peaks. */
function useOverviewPeaks(media: MediaRef | undefined): PeakData | null {
  const transport = useTransport();
  const [peaks, setPeaks] = useState<{ media: string; data: PeakData } | null>(null);
  const [version, setVersion] = useState(0);
  const id = media?.id;
  const frames = media?.frames ?? 0;
  useEffect(() => {
    if (!id) return;
    return transport.onEvent((e) => {
      if (e.type === "Media" && e.event.type === "PeaksReady" && e.event.media === id) setVersion((v) => v + 1);
    });
  }, [transport, id]);
  useEffect(() => {
    if (!id || frames <= 0) return;
    let active = true;
    let level = 16;
    while (level * OVERVIEW_PEAKS < frames) level *= 2;
    transport
      .send(
        cmd("Media", {
          type: "GetPeaks",
          request: {
            media: id,
            samples_per_peak: level,
            start_frame: 0,
            frame_count: frames,
          },
        }),
      )
      .then((reply) => {
        if (active && reply.type === "Peaks") setPeaks({ media: id, data: reply.peaks });
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [transport, id, frames, version]);
  return peaks && peaks.media === id ? peaks.data : null;
}

function Waveform({ peaks, frames }: { peaks: PeakData | null; frames: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [theme] = useTheme();
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const w = canvas.clientWidth || 600;
    const h = canvas.clientHeight || 80;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    const ctx = canvas.getContext?.("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    if (!peaks || frames <= 0) return;
    ctx.fillStyle = readToken("--eth-color-accent", canvas);
    const n = peaks.max[0]?.length ?? 0;
    const spp = peaks.samples_per_peak;
    for (let x = 0; x < w; x++) {
      const i0 = Math.floor(((x / w) * frames) / spp);
      const i1 = Math.max(i0 + 1, Math.floor((((x + 1) / w) * frames) / spp));
      let lo = 0;
      let hi = 0;
      for (let ch = 0; ch < peaks.max.length; ch++) {
        for (let i = i0; i < Math.min(i1, n); i++) {
          hi = Math.max(hi, peaks.max[ch]![i]!);
          lo = Math.min(lo, peaks.min[ch]![i]!);
        }
      }
      const y0 = ((1 - hi) / 2) * h;
      const y1 = ((1 - lo) / 2) * h;
      ctx.fillRect(x, y0, 1, Math.max(1, y1 - y0));
    }
  }, [peaks, frames, theme]);
  return <canvas ref={ref} className="eth-slices__canvas" data-testid="slice-waveform" />;
}

/** Slice markers of a sampler over its waveform, auto-slicing and "Slice to Drum Rack". */
export function SliceEditor({ sampler }: { sampler: Device }) {
  const send = useSend();
  const slices = slicesOf(sampler)!;
  const sample = sampleOf(sampler);
  const media = useProjectStore((s) => (sample ? s.project?.media[sample] : undefined));
  const tempo = useProjectStore((s) => (s.project ? Object.values(s.project.tempo_points).find((p) => p.time < 1e-9)?.bpm : undefined));
  const peaks = useOverviewPeaks(media);
  const lane = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ index: number; seconds: number } | null>(null);
  const [mode, setMode] = useState<AutoMode>("Transients");
  const [sensitivity, setSensitivity] = useState(50);
  const [grid, setGrid] = useState("1");
  const [count, setCount] = useState(8);
  const selectTrack = useSelectionStore((s) => s.selectTrack);

  const length = media && media.frames > 0 ? media.frames / media.sample_rate : 0;
  const secondsAt = (clientX: number): number => {
    const r = lane.current?.getBoundingClientRect();
    if (!r || r.width <= 0 || length <= 0) return 0;
    return Math.min(length, Math.max(0, ((clientX - r.left) / r.width) * length));
  };
  const pct = (s: number) => `${length > 0 ? (s / length) * 100 : 0}%`;

  const autoMode = (): AutoSlice =>
    mode === "Transients"
      ? { type: "Transients", sensitivity: sensitivity / 100 }
      : mode === "Grid"
        ? { type: "Grid", beats: Number(grid) }
        : { type: "Equal", count };

  const toRack = () => {
    const n = playableSlices(slices);
    if (n === 0) return;
    void send(
      cmd("Slice", {
        type: "ToDrumRack",
        device: sampler.id,
        rack: newId(),
        pads: slicePadIds(n),
      }),
    ).then(() => selectTrack(sampler.track));
  };

  const markers = slices.markers.map((m, i) => (drag?.index === i ? drag.seconds : m));
  const onMarkerDown = (e: PointerEvent, index: number) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
    setDrag({ index, seconds: slices.markers[index]! });
  };
  const onMarkerMove = (e: PointerEvent) => {
    if (drag) setDrag({ ...drag, seconds: secondsAt(e.clientX) });
  };
  const onMarkerUp = () => {
    if (!drag) return;
    const d = drag;
    setDrag(null);
    if (Math.abs(d.seconds - slices.markers[d.index]!) > 1e-6) {
      void send(
        cmd("Slice", {
          type: "Move",
          device: sampler.id,
          index: d.index,
          position: d.seconds,
        }),
      );
    }
  };

  return (
    <section className="eth-slices" aria-label={`${sampler.name} slices`} data-testid="slice-editor">
      <div className="eth-slices__toolbar">
        <Toggle
          size="sm"
          label="Slice mode"
          checked={slices.enabled}
          onChange={(enabled) => void send(cmd("Slice", { type: "SetEnabled", device: sampler.id, enabled }))}
        />
        <label className="eth-drum-rack__field">
          <span className="eth-drum-rack__label">First note</span>
          <NumberField
            size="sm"
            aria-label="Slice base note"
            value={slices.base_note}
            min={0}
            max={127}
            onChange={(note) =>
              void send(
                cmd("Slice", {
                  type: "SetBaseNote",
                  device: sampler.id,
                  note: Math.round(note),
                }),
              )
            }
          />
          <span className="eth-drum-rack__hint">{noteName(slices.base_note)}</span>
        </label>
        <Select<AutoMode> size="sm" aria-label="Auto-slice mode" options={AUTO_MODES} value={mode} onChange={setMode} />
        {mode === "Transients" && (
          <NumberField size="sm" aria-label="Sensitivity" value={sensitivity} min={0} max={100} unit="%" onChange={setSensitivity} />
        )}
        {mode === "Grid" && <Select size="sm" aria-label="Slice grid" options={GRID_OPTIONS} value={grid} onChange={setGrid} />}
        {mode === "Equal" && <NumberField size="sm" aria-label="Slice count" value={count} min={1} max={128} onChange={setCount} />}
        <Button
          size="sm"
          disabled={!media}
          onClick={() =>
            void send(
              cmd("Slice", {
                type: "Auto",
                device: sampler.id,
                mode: autoMode(),
              }),
            )
          }
          title={mode === "Grid" && tempo ? `At ${tempo.toFixed(1)} BPM` : undefined}
        >
          Auto-slice
        </Button>
        <Button size="sm" tone="accent" disabled={playableSlices(slices) === 0} onClick={toRack}>
          Slice to Drum Rack
        </Button>
        <span className="eth-drum-rack__hint" data-testid="slice-count">
          {slices.markers.length} {slices.markers.length === 1 ? "slice" : "slices"}
        </span>
      </div>
      <SampleSlot device={sampler} />
      {media && (
        <div
          ref={lane}
          className={clsx("eth-slices__lane", !slices.enabled && "eth-slices__lane--off")}
          data-testid="slice-lane"
          title="Click to add a slice; drag a marker to move it; right-click a marker to remove it"
          onPointerMove={onMarkerMove}
          onPointerUp={onMarkerUp}
          onClick={(e) => {
            if (drag || length <= 0) return;
            void send(
              cmd("Slice", {
                type: "Add",
                device: sampler.id,
                positions: [secondsAt(e.clientX)],
              }),
            );
          }}
        >
          <Waveform peaks={peaks} frames={media.frames} />
          {markers.map((m, i) => (
            <div
              key={i}
              role="slider"
              tabIndex={0}
              aria-label={`Slice ${i + 1}`}
              aria-valuemin={0}
              aria-valuemax={length}
              aria-valuenow={m}
              aria-valuetext={`${m.toFixed(3)} s`}
              className={clsx("eth-slices__marker", drag?.index === i && "eth-slices__marker--drag")}
              style={{ left: pct(m) }}
              onClick={(e) => e.stopPropagation()}
              onPointerDown={(e) => onMarkerDown(e, i)}
              onDoubleClick={(e) => {
                e.stopPropagation();
                void send(
                  cmd("Slice", {
                    type: "Remove",
                    device: sampler.id,
                    indices: [i],
                  }),
                );
              }}
              onKeyDown={(e) => {
                if (e.key === "Delete" || e.key === "Backspace")
                  void send(
                    cmd("Slice", {
                      type: "Remove",
                      device: sampler.id,
                      indices: [i],
                    }),
                  );
              }}
              onContextMenu={(e) =>
                openContextMenu(e, [
                  {
                    label: "Remove Slice",
                    danger: true,
                    onSelect: () =>
                      void send(
                        cmd("Slice", {
                          type: "Remove",
                          device: sampler.id,
                          indices: [i],
                        }),
                      ),
                  },
                ])
              }
            >
              <span className="eth-slices__marker-label">
                {slices.enabled && slices.base_note + i <= 127 ? noteName(slices.base_note + i) : i + 1}
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
