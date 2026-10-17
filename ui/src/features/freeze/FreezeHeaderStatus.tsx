import clsx from "clsx";
import { Lock, Snowflake, TriangleAlert, X } from "lucide-react";
import type { Track } from "@/generated";
import type { EngineTransport } from "@/transport";
import { useT } from "@/i18n";
import { useProjectStore } from "@/state";
import { useFrozenAncestor } from "./inert";
import { cancelRenderJob, dismissError, jobOfTrack, unfreezeTracks, useFreezeUi } from "./store";

/**
 * Freeze state in a track header: while a render of the track runs (or waits), a cancel
 * button and a progress bar along the card's bottom edge; a frozen track shows a pressed
 * snowflake (click: unfreeze); a failed render shows a warning (click: dismiss). A track
 * inside a frozen group shows a lock (its group plays one render; unfreeze the group first).
 */
export function FreezeHeaderStatus({ track, transport }: { track: Track; transport: EngineTransport }) {
  const job = useFreezeUi((s) => jobOfTrack(s, track.id));
  const error = useFreezeUi((s) => s.errors[track.id]);
  const owner = useFrozenAncestor(track.id);
  // Partial freeze (#25): the freeze point's name.
  const upTo = useProjectStore((s) => (track.freeze?.up_to ? s.project?.devices[track.freeze.up_to]?.name : undefined));
  const t = useT();
  if (job) {
    const pct = job.progress === null ? null : Math.round(job.progress * 100);
    const kind = job.kind.toLowerCase();
    const what = t("freeze.status.what", { kind, name: track.name });
    return (
      <>
        <button
          type="button"
          className="eth-arr-header__toggle eth-freeze__cancel"
          aria-label={t("freeze.status.cancel", { kind, name: track.name })}
          title={pct === null ? t("freeze.status.queued", { what }) : t("freeze.status.progress", { what, pct })}
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
    const group = track.kind === "Group";
    return (
      <button
        type="button"
        className="eth-arr-header__toggle eth-freeze__toggle"
        aria-pressed
        aria-label={t("freeze.status.unfreeze", { name: track.name })}
        title={
          upTo !== undefined ? t("freeze.status.frozen_partial", { device: upTo }) : group ? t("freeze.status.frozen_group") : t("freeze.status.frozen_track")
        }
        onClick={(e) => {
          e.stopPropagation();
          void unfreezeTracks(transport, [track.id]);
        }}
      >
        <Snowflake />
      </button>
    );
  }
  if (owner) {
    return (
      <span
        className="eth-arr-header__toggle eth-freeze__lock"
        role="img"
        aria-label={t("freeze.status.locked", { name: owner.name })}
        title={t("freeze.status.locked_title", { name: owner.name })}
      >
        <Lock />
      </span>
    );
  }
  if (error) {
    return (
      <button
        type="button"
        className="eth-arr-header__toggle eth-freeze__error"
        aria-label={t("freeze.status.failed", { error })}
        title={t("freeze.status.failed_title", { error })}
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
