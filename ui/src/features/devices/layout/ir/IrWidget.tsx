/**
 * The Convolution Reverb's impulse response widget (`fx-space`; drawn for the device's
 * `SampleWaveform` slot): an IR picker (factory IRs by category, the project media in use,
 * previous/next), a drop target for audio (browser items or files: imported, then
 * `Device::SetIr`), and the IR as the reverb plays it (Pre-delay, Size, Decay, Reverse) with
 * draggable Pre-delay and Decay markers. Missing media offer Relink.
 */

import clsx from "clsx";
import { ChevronLeft, ChevronRight, X } from "lucide-react";
import { useRef, useState, type DragEvent } from "react";
import type { IrSource, MediaRef, WidgetSize } from "@/generated";
import { hasBrowserDrag, readBrowserDrag, resolveDroppedMedia } from "@/features/browser/dragPayload";
import { droppedFiles, hasOsFiles, importAudio, noteHover, type ImportSource } from "@/features/import";
import { openRelink, useMediaMissing } from "@/features/media-refs";
import { Button, IconButton, Select, type SelectOption } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, nextGestureId, useTransport } from "@/transport";
import { useLayoutContext, useParam, type ParamBinding } from "../context";
import { GridLines, Plot } from "../plot";
import { useOverviewPeaks } from "../widgets/data";
import { TypedFrame } from "../widgets/typed";
import { useFactoryIrs } from "./factoryIrs";
import {
  IR_PARAMS,
  axisSeconds,
  decayAt,
  envelopePath,
  factoryEnvelope,
  formatSeconds,
  irFromKey,
  irKey,
  peaksEnvelope,
  preDelayAt,
  shapedLength,
  shapingOf,
  stepFactory,
  timeToX,
  type Envelope,
} from "./irMath";
import "./ir.css";

/** Pointer distance (px) that grabs a marker. */
const HIT = 14;

function irOf(kind: { type: string; device?: unknown }): IrSource | null {
  const k = kind as { type: string; device?: { type: string; ir?: IrSource | null } };
  return k.type === "Builtin" && k.device?.type === "ConvolutionReverb" ? (k.device.ir ?? null) : null;
}

function plainOf(b: ParamBinding | null, fallback: number): number {
  return b ? b.plain : fallback;
}

export function IrWidget({ size, label }: { size: WidgetSize; label: string | null }) {
  const { device, sender } = useLayoutContext();
  const transport = useTransport();
  const ir = irOf(device.kind);
  const factory = useFactoryIrs(transport);
  const mediaId = ir?.type === "Media" ? ir.media : null;
  const media: MediaRef | undefined = useProjectStore((s) => (mediaId ? s.project?.media[mediaId] : undefined));
  const missing = useMediaMissing(mediaId);
  const peaks = useOverviewPeaks(media);
  const decay = useParam(IR_PARAMS.decay);
  const stretch = useParam(IR_PARAMS.size);
  const reverse = useParam(IR_PARAMS.reverse);
  const pre = useParam(IR_PARAMS.preDelay);
  const [over, setOver] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const grab = useRef<"decay" | "pre" | null>(null);

  const shaping = shapingOf({
    decay: plainOf(decay, 100),
    size: plainOf(stretch, 100),
    reverse: plainOf(reverse, 0),
    preDelay: plainOf(pre, 0),
  });
  const spec = ir?.type === "Factory" ? factory.find((f) => f.id === ir.id) : undefined;
  const length = spec ? spec.length : media && media.sample_rate > 0 ? media.frames / media.sample_rate : 0;
  const env: Envelope | null = spec ? factoryEnvelope(spec) : peaks && media ? peaksEnvelope(peaks, media.sample_rate) : null;

  const setIr = async (next: IrSource | null, opts: { gesture?: ReturnType<typeof nextGestureId> } = {}) => {
    setError(null);
    try {
      await transport.send(cmd("Device", { type: "SetIr", device: device.id, ir: next }), opts);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const options: SelectOption<string>[] = [{ value: "none", label: "No impulse response" }];
  for (const f of factory) options.push({ value: `factory:${f.id}`, label: f.name, group: f.category });
  if (ir?.type === "Media") options.push({ value: irKey(ir), label: media?.name ?? "Audio file", group: "Project" });

  const importFiles = async (sources: ImportSource[]) => {
    const outcomes = await importAudio(transport, sources.slice(0, 1));
    const done = outcomes.find((o) => o.media);
    const failed = outcomes.find((o) => o.error && o.error !== "cancelled");
    if (failed) setError(failed.error);
    if (done?.media) await setIr({ type: "Media", media: done.media.id });
  };

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    const browser = hasBrowserDrag(e.dataTransfer);
    const files = hasOsFiles(e.dataTransfer);
    if (!browser && !files) return;
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = "copy";
    setOver(true);
    if (files) noteHover(e, (sources) => void importFiles(sources));
  };

  const onDrop = async (e: DragEvent<HTMLDivElement>) => {
    setOver(false);
    setError(null);
    if (hasOsFiles(e.dataTransfer)) {
      e.preventDefault();
      e.stopPropagation();
      await importFiles(droppedFiles(e.dataTransfer)).catch((err: unknown) => setError(String(err)));
      return;
    }
    const payload = readBrowserDrag(e.dataTransfer);
    if (!payload) return;
    e.preventDefault();
    e.stopPropagation();
    const gesture = nextGestureId();
    try {
      const m = await resolveDroppedMedia(transport, payload, { gesture });
      await setIr({ type: "Media", media: m.id }, { gesture });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
    }
  };

  const { kept, stretched } = shapedLength(length, shaping);
  const name = spec?.name ?? media?.name ?? null;
  const channels = spec?.channels ?? media?.channels ?? 0;
  const meta = length > 0 ? `${formatSeconds(kept)}${kept < stretched - 1e-6 ? ` of ${formatSeconds(stretched)}` : ""} · ${channels === 1 ? "Mono" : "Stereo"}` : "";
  const describe = name ? `Impulse response ${name}, ${meta}${shaping.reverse ? ", reversed" : ""}` : "No impulse response";
  const hint = error
    ? `Load failed: ${error}`
    : missing
      ? "The IR file is missing: the reverb passes the dry signal through."
      : "Drop audio from the Browser or your files to use it as the impulse response.";

  return (
    <TypedFrame type="ir" size={size} label={label}>
      <div
        className={clsx("eth-ir", over && "eth-ir--over", !ir && "eth-ir--empty")}
        data-testid="ir-widget"
        onDragOver={onDragOver}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => void onDrop(e)}
      >
        <div className="eth-ir__bar">
          <IconButton size="sm" tone="ghost" label="Previous impulse response" icon={<ChevronLeft />} onClick={() => void setIr(stepFactory(factory, ir, -1))} />
          <Select
            className="eth-ir__select"
            size="sm"
            options={options}
            value={irKey(ir)}
            aria-label="Impulse response"
            data-testid="ir-select"
            onChange={(v) => {
              const next = irFromKey(v);
              if (next !== undefined) void setIr(next);
            }}
          />
          <IconButton size="sm" tone="ghost" label="Next impulse response" icon={<ChevronRight />} onClick={() => void setIr(stepFactory(factory, ir, 1))} />
          {meta && (
            <span className="eth-ir__meta" data-testid="ir-meta">
              {meta}
            </span>
          )}
          {ir && <IconButton size="sm" tone="ghost" label="Remove impulse response" icon={<X />} onClick={() => void setIr(null)} />}
        </div>
        <Plot
          className="eth-plot--ir"
          label={describe}
          testId="widget-ir"
          begin={sender.begin}
          end={sender.end}
          drag={
            env && length > 0
              ? {
                  start: (x, _y, e) => {
                    const w = e.currentTarget.getBoundingClientRect().width || 1;
                    const dEnd = Math.abs(timeToX(shaping.preDelay + kept, length, w) - x);
                    const dPre = Math.abs(timeToX(shaping.preDelay, length, w) - x);
                    if (decay && dEnd <= HIT && dEnd <= dPre) grab.current = "decay";
                    else if (pre && dPre <= HIT) grab.current = "pre";
                    else return false;
                  },
                  move: (x, _y, e) => {
                    const w = e.currentTarget.getBoundingClientRect().width || 1;
                    if (grab.current === "decay") decay?.setPlain(decayAt(x, length, shaping, w));
                    else if (grab.current === "pre") pre?.setPlain(preDelayAt(x, length, w));
                  },
                  end: () => {
                    grab.current = null;
                  },
                }
              : undefined
          }
        >
          {({ w, h }) => {
            if (!env || length <= 0) {
              return (
                <>
                  <GridLines w={w} h={h} ys={[0.5]} />
                  <text className="eth-plot__empty" x={w / 2} y={h / 2}>
                    {ir ? (missing ? "Missing file" : "Loading…") : "No impulse response"}
                  </text>
                </>
              );
            }
            const axis = axisSeconds(length);
            const step = axis > 6 ? 2 : axis > 2 ? 1 : axis > 1 ? 0.5 : 0.1;
            const ticks: number[] = [];
            for (let t = step; t < axis; t += step) ticks.push(t);
            const xPre = timeToX(shaping.preDelay, length, w);
            const xEnd = timeToX(shaping.preDelay + kept, length, w);
            const xStretched = timeToX(shaping.preDelay + stretched, length, w);
            return (
              <>
                <GridLines w={w} h={h} ys={[0.5]} xs={ticks.map((t) => t / axis)} />
                {ticks.map((t) => (
                  <text key={t} className="eth-plot__empty eth-ir__tick" x={(t / axis) * w} y={h}>
                    {`${Number(t.toFixed(1))} s`}
                  </text>
                ))}
                <path className="eth-ir__env" d={envelopePath(env, length, shaping, w, h)} data-testid="ir-envelope" />
                {xStretched > xEnd + 0.5 && <rect className="eth-plot__dim" x={xEnd} y={0} width={xStretched - xEnd} height={h} />}
                {shaping.preDelay > 0 && <line className="eth-plot__marker eth-ir__pre" x1={xPre} x2={xPre} y1={0} y2={h} data-handle="Pre-delay" />}
                <line className="eth-plot__marker" x1={xEnd} x2={xEnd} y1={0} y2={h} data-handle="Decay" />
                <circle className="eth-plot__handle" cx={xEnd} cy={h / 2} />
                {shaping.reverse && (
                  <text className="eth-plot__empty eth-ir__badge" x={w} y={0}>
                    Reversed
                  </text>
                )}
              </>
            );
          }}
        </Plot>
        <div className={clsx("eth-ir__hint", (error || missing) && "eth-ir__hint--warn")} data-testid="ir-hint">
          <span className="eth-ir__hint-text">{hint}</span>
          {missing && mediaId && (
            <Button size="sm" tone="ghost" onClick={() => openRelink(mediaId)}>
              Relink…
            </Button>
          )}
        </div>
      </div>
    </TypedFrame>
  );
}
