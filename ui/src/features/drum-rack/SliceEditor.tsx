/**
 * The Sampler's slice mode, in its own window (the waveform widget of its panel): the
 * "classic | slice" switch, and the slice editor that takes the waveform's place in slice
 * mode. Slice `i` plays on key `base_note + i` (`Slice::*`, CONTRACTS.md §11.12). Every edit
 * is one undo step.
 */
import "./drumRack.css";
import clsx from "clsx";
import { useEffect, useRef, useState, type MouseEvent, type PointerEvent } from "react";
import type { AutoSlice, Device, PeakData } from "@/generated";
import { reportFailure } from "@/features/devices/gesture";
import { useOverviewPeaks } from "@/features/devices/overviewPeaks";
import { useDeviceWindows } from "@/features/devices/window/store";
import { holdNote } from "@/features/piano-roll/notePreview";
import { errorMessage } from "@/features/transport-bar/engine";
import { formatFixed, t as tr, useT, type MessageKey } from "@/i18n";
import { Button, NumberField, openContextMenu, readToken, Select, Tabs } from "@/kit";
import { useThemeKey } from "@/theme";
import { useProjectStore } from "@/state";
import { cmd, newId, useTransport } from "@/transport";
import { noteName, playableSlices, sampleOf, slicePadIds, slicesOf } from "./padUtils";
import { DEFAULT_AUTO_SLICE, sendWithPeaks } from "./sliceActions";

/** Velocity of a slice played from the editor (MIDI). */
const PLAY_VELOCITY = 100;

type AutoMode = AutoSlice["type"];
const AUTO_MODES: { value: AutoMode; label: MessageKey }[] = [
  { value: "Transients", label: "drum_rack.slices.mode_transients" },
  { value: "Grid", label: "drum_rack.slices.mode_grid" },
  { value: "Equal", label: "drum_rack.slices.mode_equal" },
];
const gridOptions = () => [
  { value: "0.25", label: "1/16" },
  { value: "0.5", label: "1/8" },
  { value: "1", label: "1/4" },
  { value: "2", label: "1/2" },
  { value: "4", label: tr("drum_rack.slices.one_bar") },
];

type SamplerMode = "classic" | "slice";
const MODES: { id: SamplerMode; label: MessageKey }[] = [
  { id: "classic", label: "drum_rack.slices.classic" },
  { id: "slice", label: "drum_rack.slices.slice" },
];

/**
 * The Sampler's mode: classic (the start–end range, played by every key) or slice. Turning
 * slice mode on for a sample without slices finds its transients in the same step.
 */
export function SamplerModeSwitch({ sampler }: { sampler: Device }) {
  const t = useT();
  const transport = useTransport();
  const slices = slicesOf(sampler);
  const sample = sampleOf(sampler);
  if (!slices) return null;
  const set = (mode: SamplerMode) => {
    const enabled = mode === "slice";
    if (enabled === slices.enabled) return;
    const toggle = () => void reportFailure(transport.send(cmd("Slice", { type: "SetEnabled", device: sampler.id, enabled })));
    if (!enabled || slices.markers.length > 0 || !sample) return toggle();
    // `Auto` turns slice mode on too: one step. Without peaks in time, just switch.
    sendWithPeaks(transport, cmd("Slice", { type: "Auto", device: sampler.id, mode: DEFAULT_AUTO_SLICE }), sample).catch(toggle);
  };
  return (
    <Tabs<SamplerMode>
      label={t("drum_rack.slices.sampler_mode")}
      items={MODES.map((m) => ({ id: m.id, label: t(m.label) }))}
      value={slices.enabled ? "slice" : "classic"}
      onChange={set}
    />
  );
}

function Waveform({ peaks, frames }: { peaks: PeakData | null; frames: number }) {
  const t = useT();
  const ref = useRef<HTMLCanvasElement>(null);
  const theme = useThemeKey();
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
  // A picture of the sample (the slice markers over it are elements with their own labels).
  return <canvas ref={ref} className="eth-slices__canvas" data-testid="slice-waveform" role="img" aria-label={t("drum_rack.slices.waveform")} />;
}

/**
 * Slice markers over the sampler's waveform: auto-slicing (transients with a sensitivity,
 * beat grid, equal parts), the first slice's key, markers added (double-click), moved (drag)
 * and removed (double-click, Delete, right-click), a slice played by pressing it, and
 * "slice to drum rack".
 */
export function SliceEditor({ sampler }: { sampler: Device }) {
  const t = useT();
  const transport = useTransport();
  const slices = slicesOf(sampler)!;
  const sample = sampleOf(sampler);
  const media = useProjectStore((s) => (sample ? s.project?.media[sample] : undefined));
  const tempo = useProjectStore((s) => (s.project ? Object.values(s.project.tempo_points).find((p) => p.time < 1e-9)?.bpm : undefined));
  const peaks = useOverviewPeaks(media);
  const lane = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ index: number; seconds: number } | null>(null);
  const [playing, setPlaying] = useState<number | null>(null);
  const [mode, setMode] = useState<AutoMode>("Transients");
  const [sensitivity, setSensitivity] = useState(50);
  const [grid, setGrid] = useState("1");
  const [count, setCount] = useState(8);
  const [error, setError] = useState<string | null>(null);

  const send = (c: Parameters<typeof transport.send>[0]) => {
    setError(null);
    transport.send(c).catch((e: unknown) => setError(errorMessage(e)));
  };
  const length = media && media.frames > 0 ? media.frames / media.sample_rate : 0;
  const secondsAt = (clientX: number): number => {
    const r = lane.current?.getBoundingClientRect();
    if (!r || r.width <= 0 || length <= 0) return 0;
    return Math.min(length, Math.max(0, ((clientX - r.left) / r.width) * length));
  };
  const pct = (s: number) => `${length > 0 ? (s / length) * 100 : 0}%`;
  // Pad samplers get `PAD_PLAY_NOTE` whatever the key: only a sampler off the pads plays slices.
  const onPad = sampler.pad !== null;

  const autoMode = (): AutoSlice =>
    mode === "Transients"
      ? { type: "Transients", sensitivity: sensitivity / 100 }
      : mode === "Grid"
        ? { type: "Grid", beats: Number(grid) }
        : { type: "Equal", count };
  const autoSlice = () => {
    if (!sample) return;
    setError(null);
    sendWithPeaks(transport, cmd("Slice", { type: "Auto", device: sampler.id, mode: autoMode() }), sample).catch((e: unknown) => setError(errorMessage(e)));
  };
  const add = (seconds: number) => send(cmd("Slice", { type: "Add", device: sampler.id, positions: [seconds] }));
  const remove = (index: number) => send(cmd("Slice", { type: "Remove", device: sampler.id, indices: [index] }));

  const toRack = () => {
    const n = playableSlices(slices);
    if (n === 0) return;
    const rack = newId();
    setError(null);
    transport
      .send(cmd("Slice", { type: "ToDrumRack", device: sampler.id, rack, pads: slicePadIds(n) }))
      // The sampler (and its window) is gone: the new rack's window shows the pads.
      .then(() => useDeviceWindows.getState().setOpen(rack, true))
      .catch((e: unknown) => setError(errorMessage(e)));
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
    if (Math.abs(d.seconds - slices.markers[d.index]!) > 1e-6) send(cmd("Slice", { type: "Move", device: sampler.id, index: d.index, position: d.seconds }));
  };

  /** Press a slice: its key, held until the pointer is released (through the track's chain). */
  const play = (e: PointerEvent, index: number) => {
    if (e.button !== 0 || onPad || slices.base_note + index > 127) return;
    holdNote(transport, sampler.track, slices.base_note + index, PLAY_VELOCITY);
    setPlaying(index);
    window.addEventListener("pointerup", () => setPlaying(null), { once: true });
  };
  const laneMenu = (e: MouseEvent, index: number | null) => {
    const at = secondsAt(e.clientX);
    openContextMenu(e, [
      { label: t("drum_rack.slices.add_here"), disabled: length <= 0, onSelect: () => add(at) },
      ...(index === null ? [] : [{ label: t("drum_rack.slices.remove"), danger: true, onSelect: () => remove(index) }]),
    ]);
  };

  return (
    <section className="eth-slices" aria-label={t("drum_rack.slices.label", { name: sampler.name })} data-testid="slice-editor">
      <div className="eth-slices__toolbar">
        <Select<AutoMode>
          size="sm"
          aria-label={t("drum_rack.slices.auto_mode")}
          options={AUTO_MODES.map((o) => ({ value: o.value, label: t(o.label) }))}
          value={mode}
          onChange={setMode}
        />
        {mode === "Transients" && (
          <NumberField size="sm" aria-label={t("drum_rack.slices.sensitivity")} value={sensitivity} min={0} max={100} unit="%" onChange={setSensitivity} />
        )}
        {mode === "Grid" && <Select size="sm" aria-label={t("drum_rack.slices.grid")} options={gridOptions()} value={grid} onChange={setGrid} />}
        {mode === "Equal" && <NumberField size="sm" aria-label={t("drum_rack.slices.count_field")} value={count} min={1} max={128} onChange={setCount} />}
        <Button
          size="sm"
          disabled={!media}
          onClick={autoSlice}
          title={mode === "Grid" && tempo ? t("drum_rack.slices.at_tempo", { bpm: formatFixed(tempo, 1) }) : undefined}
        >
          {t("drum_rack.slices.auto")}
        </Button>
        <label className="eth-drum-rack__field">
          <span className="eth-drum-rack__label">{t("drum_rack.slices.first_key")}</span>
          <NumberField
            size="sm"
            aria-label={t("drum_rack.slices.first_key_label")}
            value={slices.base_note}
            min={0}
            max={127}
            onChange={(note) => send(cmd("Slice", { type: "SetBaseNote", device: sampler.id, note: Math.round(note) }))}
          />
          <span className="eth-drum-rack__hint eth-user-text">{noteName(slices.base_note)}</span>
        </label>
        <span className="eth-drum-rack__hint" data-testid="slice-count">
          {t("drum_rack.slices.count", { count: slices.markers.length })}
        </span>
        <Button size="sm" tone="accent" disabled={playableSlices(slices) === 0 || onPad} onClick={toRack} title={t("drum_rack.slices.to_rack_title")}>
          {t("drum_rack.slice.to_drum_rack")}
        </Button>
      </div>
      {error && (
        <p className="eth-drum-rack__hint" role="alert">
          {error}
        </p>
      )}
      {media ? (
        <div
          ref={lane}
          className="eth-slices__lane"
          data-testid="slice-lane"
          title={t("drum_rack.slices.lane_title")}
          onPointerMove={onMarkerMove}
          onPointerUp={onMarkerUp}
          onDoubleClick={(e) => {
            if (length > 0) add(secondsAt(e.clientX));
          }}
          onContextMenu={(e) => laneMenu(e, null)}
        >
          <Waveform peaks={peaks} frames={media.frames} />
          {markers.map((m, i) => (
            <div
              key={`region-${i}`}
              role="button"
              tabIndex={-1}
              aria-label={t("drum_rack.slices.play", { n: i + 1 })}
              aria-pressed={playing === i}
              className={clsx("eth-slices__region", playing === i && "eth-slices__region--playing", slices.base_note + i > 127 && "eth-slices__region--off")}
              style={{ left: pct(m), width: pct(Math.max(0, (markers[i + 1] ?? length) - m)) }}
              data-testid="slice-region"
              onPointerDown={(e) => play(e, i)}
              onContextMenu={(e) => {
                e.stopPropagation();
                laneMenu(e, i);
              }}
            />
          ))}
          {markers.map((m, i) => (
            <div
              key={i}
              role="slider"
              tabIndex={0}
              aria-label={t("drum_rack.slices.marker", { n: i + 1 })}
              aria-valuemin={0}
              aria-valuemax={length}
              aria-valuenow={m}
              aria-valuetext={`${m.toFixed(3)} s`}
              className={clsx("eth-slices__marker", drag?.index === i && "eth-slices__marker--drag")}
              style={{ left: pct(m) }}
              onPointerDown={(e) => onMarkerDown(e, i)}
              onDoubleClick={(e) => {
                e.stopPropagation();
                remove(i);
              }}
              onKeyDown={(e) => {
                if (e.key === "Delete" || e.key === "Backspace") remove(i);
              }}
              onContextMenu={(e) => {
                e.stopPropagation();
                openContextMenu(e, [{ label: t("drum_rack.slices.remove"), danger: true, onSelect: () => remove(i) }]);
              }}
            >
              <span className="eth-slices__marker-label eth-user-text">{slices.base_note + i <= 127 ? noteName(slices.base_note + i) : i + 1}</span>
            </div>
          ))}
        </div>
      ) : (
        <p className="eth-drum-rack__hint">{t("drum_rack.slices.load_sample")}</p>
      )}
    </section>
  );
}
