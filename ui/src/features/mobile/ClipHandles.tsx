/**
 * The trim handles of the selected clips on touch screens (#522): a grip pill on each edge,
 * drawn over the lanes (the arrangement's overlay, lanes coordinates). Their touch targets are
 * wider than drawn (`clipTouch.ts`: 44 px). Mobile UI only; nothing on the desktop.
 */
import type { Row } from "@/features/arrangement/layout";
import { clipRects } from "@/features/arrangement/layout";
import { arrangementView } from "@/features/arrangement/uiStore";
import { useProjectStore, useReadOnly } from "@/state";
import { itemSelection, useTimelineView } from "@/timeline";
import { useStore } from "zustand";
import { trimZones } from "./clipTouch";
import { useMobileUi } from "./mode";
import { useTouchDrag } from "./dragState";

/** More selected clips than this show no handles (a big selection is moved, not trimmed). */
const MAX_HANDLED = 8;

export function ClipHandles({ rows }: { rows: ReadonlyArray<Row> }) {
  // A read-only view trims nothing: no handles.
  const readOnly = useReadOnly();
  const mobile = useMobileUi((s) => s.enabled) && !readOnly;
  const selected = useStore(itemSelection, (s) => s.selected.clip);
  const clips = useProjectStore((s) => s.project?.clips);
  // Follows pans and zooms (only while something is selected: a few elements).
  const pxPerBeat = useTimelineView(arrangementView, (s) => (mobile && selected.size > 0 ? s.pxPerBeat : 0));
  const scrollBeats = useTimelineView(arrangementView, (s) => (mobile && selected.size > 0 ? s.scrollBeats : 0));
  // Hidden while a clip is dragged (the live preview moves, the clips don't yet).
  const dragging = useTouchDrag((s) => s.point !== null);
  if (!mobile || !clips || dragging || selected.size === 0 || selected.size > MAX_HANDLED) return null;
  const chosen = [...selected].flatMap((id) => (clips[id] ? [clips[id]] : []));
  const boxes = clipRects(rows, chosen, { pxPerBeat, scrollBeats }, 0);
  return (
    <>
      {boxes.map(({ id, rect }) =>
        trimZones(rect) ? (
          <span
            key={id}
            className="eth-clip-handles"
            data-testid="clip-handles"
            style={{ left: rect.x0, top: rect.y0, width: rect.x1 - rect.x0, height: rect.y1 - rect.y0 }}
          >
            <span className="eth-clip-handles__grip eth-clip-handles__grip--start" />
            <span className="eth-clip-handles__grip eth-clip-handles__grip--end" />
          </span>
        ) : null,
      )}
    </>
  );
}
