import { useEffect, useState } from "react";
import type { MediaRef, PeakData } from "@/generated";
import { cmd, useTransport } from "@/transport";

/** Peaks fetched for the overview. */
const OVERVIEW_PEAKS = 1024;

/**
 * Whole-sample min/max peaks of `media` (about `OVERVIEW_PEAKS` columns), refetched when the
 * engine reports new peaks: the Sampler's waveform and its slice editor.
 */
export function useOverviewPeaks(media: MediaRef | undefined): PeakData | null {
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
      .send(cmd("Media", { type: "GetPeaks", request: { media: id, samples_per_peak: level, start_frame: 0, frame_count: frames } }))
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
