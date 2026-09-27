import clsx from "clsx";
import { Snowflake, TriangleAlert, X } from "lucide-react";
import type { Track } from "@/generated";
import type { EngineTransport } from "@/transport";
import { cancelRenderJob, dismissError, jobOfTrack, unfreezeTracks, useFreezeUi } from "./store";

const VERB = { Freeze: "Freezing", Bounce: "Bouncing", Consolidate: "Consolidating" } as const;

/**
 * Freeze state in a track header: while a render of the track runs (or waits), a cancel
 * button and a progress bar along the card's bottom edge; a frozen track shows a pressed
 * snowflake (click: unfreeze); a failed render shows a warning (click: dismiss).
 */
export function FreezeHeaderStatus({ track, transport }: { track: Track; transport: EngineTransport }) {
  const job = useFreezeUi((s) => jobOfTrack(s, track.id));
  const error = useFreezeUi((s) => s.errors[track.id]);
  if (job) {
    const pct = job.progress === null ? null : Math.round(job.progress * 100);
    const what = `${VERB[job.kind]} ${track.name}`;
    return (
      <>
        <button
          type="button"
          className="eth-arr-header__toggle eth-freeze__cancel"
          aria-label={`Cancel ${job.kind.toLowerCase()} of ${track.name}`}
          title={pct === null ? `${what}: queued (click to cancel)` : `${what}: ${pct}% (click to cancel)`}
          onClick={(e) => {
            e.stopPropagation();
            void cancelRenderJob(transport, job);
          }}
        >
          <X />
        </button>
        <span
          className={clsx("eth-freeze__progress", pct === null && "eth-freeze__progress--queued")}
          role="progressbar"
          aria-label={what}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct ?? undefined}
        >
          <span className="eth-freeze__progress-fill" style={{ transform: `scaleX(${job.progress ?? 0})` }} />
        </span>
      </>
    );
  }
  if (track.freeze) {
    return (
      <button
        type="button"
        className="eth-arr-header__toggle eth-freeze__toggle"
        aria-pressed
        aria-label={`Unfreeze ${track.name}`}
        title="Frozen: the track plays its render, its clips and devices are locked (click to unfreeze)"
        onClick={(e) => {
          e.stopPropagation();
          void unfreezeTracks(transport, [track.id]);
        }}
      >
        <Snowflake />
      </button>
    );
  }
  if (error) {
    return (
      <button
        type="button"
        className="eth-arr-header__toggle eth-freeze__error"
        aria-label={`Render failed: ${error}`}
        title={`${error} (click to dismiss)`}
        onClick={(e) => {
          e.stopPropagation();
          dismissError(track.id);
        }}
      >
        <TriangleAlert />
      </button>
    );
  }
  return null;
}
