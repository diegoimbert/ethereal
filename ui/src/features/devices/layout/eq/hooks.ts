/**
 * Data hooks of the `EqCurve` widget: the device's pre/post spectrum and the engine sample
 * rate (the response math uses it, CONTRACTS.md §12.15).
 */

import { useEffect, useState } from "react";
import type { AnalysisData, DeviceId } from "@/generated";
import { useAudioSettings } from "@/features/audio-settings/store";
import { cmd, useTransport } from "@/transport";
import type { AnalysisModule } from "../seams";

type Spectrum = Extract<AnalysisData, { type: "Spectrum" }>;

export interface EqSpectra {
  pre: Spectrum | null;
  post: Spectrum | null;
}

const NONE: EqSpectra = { pre: null, post: null };

// `fx-analysis`'s shared hook when present (same glob as `../seams.ts`); until then the
// widget watches the device itself (identical semantics: latest frame per stage, watched
// while mounted).
const shared = Object.values(import.meta.glob<AnalysisModule>("../../analysis/index.ts", { eager: true }))[0]?.useDeviceAnalysis;

function useLocalAnalysis(device: DeviceId, enabled: boolean): EqSpectra {
  const transport = useTransport();
  const [state, setState] = useState<{ device: DeviceId; spectra: EqSpectra } | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let pending: EqSpectra | null = null;
    let raf = 0;
    const flush = () => {
      raf = 0;
      if (pending) setState({ device, spectra: pending });
    };
    let latest = NONE;
    const off = transport.onEvent((e) => {
      if (e.type !== "Analysis" || e.event.device !== device || e.event.data.type !== "Spectrum") return;
      const data = e.event.data;
      latest = data.stage === "Pre" ? { ...latest, pre: data } : { ...latest, post: data };
      pending = latest;
      if (!raf) raf = typeof requestAnimationFrame === "undefined" ? (flush(), 0) : requestAnimationFrame(flush);
    });
    transport.send(cmd("Analysis", { type: "Watch", device })).catch(() => {});
    return () => {
      off();
      if (raf) cancelAnimationFrame(raf);
      transport.send(cmd("Analysis", { type: "Unwatch", device })).catch(() => {});
    };
  }, [transport, device, enabled]);
  return enabled && state?.device === device ? state.spectra : NONE;
}

function useSharedAnalysis(device: DeviceId, enabled: boolean): EqSpectra {
  const frames = shared!(device);
  if (!enabled) return NONE;
  let pre: Spectrum | null = null;
  let post: Spectrum | null = null;
  for (const f of frames) {
    if (f.type !== "Spectrum") continue;
    if (f.stage === "Pre") pre = f;
    else post = f;
  }
  return { pre, post };
}

/** The device's latest input (`pre`) and output (`post`) spectrum while mounted. */
export const useEqSpectra: (device: DeviceId, enabled: boolean) => EqSpectra = shared ? useSharedAnalysis : useLocalAnalysis;

/** Fallback when the engine's rate is unknown (CONTRACTS.md §12.15). */
export const DEFAULT_SAMPLE_RATE = 48000;

let knownRate: number | null = null;

/** The engine sample rate (`EngineStatus`), else 48 kHz. */
export function useEngineSampleRate(): number {
  const transport = useTransport();
  const fromSettings = useAudioSettings((s) => s.status?.sample_rate ?? null);
  const [rate, setRate] = useState<number | null>(knownRate);
  useEffect(() => {
    let active = true;
    const off = transport.onEvent((e) => {
      if (e.type === "Engine" && e.event.type === "Status" && e.event.status.sample_rate > 0) {
        knownRate = e.event.status.sample_rate;
        setRate(knownRate);
      }
    });
    if (knownRate === null) {
      transport
        .send(cmd("Engine", { type: "GetStatus" }))
        .then((reply) => {
          if (active && reply.type === "Status" && reply.status.sample_rate > 0) {
            knownRate = reply.status.sample_rate;
            setRate(knownRate);
          }
        })
        .catch(() => {});
    }
    return () => {
      active = false;
      off();
    };
  }, [transport]);
  const r = fromSettings && fromSettings > 0 ? fromSettings : rate;
  return r && r > 0 ? r : DEFAULT_SAMPLE_RATE;
}
