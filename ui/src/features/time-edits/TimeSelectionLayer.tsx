import "./time-edits.css";
import { useEffect } from "react";
import type { TrackId } from "@/generated";
import { useViewport, type TimelineViewStore } from "@/timeline";
import { useTimeSelection } from "./store";

interface LayerRow {
  track: { id: TrackId };
  y: number;
  height: number;
}

/**
 * The time selection drawn over the lanes of its tracks (content px, lanes start at
 * `headerWidth`), with its edges. Pointer events pass through.
 */
export function TimeSelectionLayer({
  rows,
  view,
  headerWidth,
}: {
  rows: ReadonlyArray<LayerRow>;
  view: TimelineViewStore;
  headerWidth: number;
}) {
  const sel = useTimeSelection((s) => s.selection);
  const vp = useViewport(view);
  if (!sel) return null;
  const left = headerWidth + (sel.start - vp.scrollBeats) * vp.pxPerBeat;
  const width = (sel.end - sel.start) * vp.pxPerBeat;
  const shown = rows.filter((r) => sel.tracks.includes(r.track.id));
  return (
    <>
      {shown.map((r) => (
        <div
          key={r.track.id}
          className="eth-time-sel"
          data-testid="time-selection"
          data-track={r.track.id}
          style={{
            left: Math.max(left, headerWidth),
            top: r.y,
            width: Math.max(0, left + width - Math.max(left, headerWidth)),
            height: r.height,
          }}
        />
      ))}
    </>
  );
}

/** How long a refused-edit notice stays (ms). */
const NOTICE_MS = 4000;

/** The last refused time edit (e.g. "track "Bass" is frozen: …"), shown for a few seconds. */
export function TimeEditNotice() {
  const notice = useTimeSelection((s) => s.notice);
  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(() => {
      if (useTimeSelection.getState().notice?.id === notice.id) useTimeSelection.getState().notify(null);
    }, NOTICE_MS);
    return () => clearTimeout(t);
  }, [notice]);
  if (!notice) return null;
  return (
    <div className="eth-time-notice" role="status" data-testid="time-edit-notice" onClick={() => useTimeSelection.getState().notify(null)}>
      {notice.message}
    </div>
  );
}
