/**
 * The time range's handles on touch screens: a grip on each end of the range, drawn over
 * the lanes (the arrangement's overlay, lanes coordinates). Their touch targets are wider than
 * drawn (`rangeTouch.ts`). Mobile UI only; nothing on the desktop.
 */
import type { Row } from "@/features/arrangement/layout";
import { arrangementView } from "@/features/arrangement/uiStore";
import { trackRangeOf, useTimeSelection } from "@/features/time-edits";
import { useTimelineView } from "@/timeline";
import { useMobileUi } from "./mode";
import { rangeBox } from "./rangeTouch";

export function RangeHandles({ rows }: { rows: ReadonlyArray<Row> }) {
  const mobile = useMobileUi((s) => s.enabled);
  const range = useTimeSelection((s) => (mobile ? trackRangeOf(s.selection) : null));
  // Follows pans and zooms (only while a range is shown).
  const pxPerBeat = useTimelineView(arrangementView, (s) => (range ? s.pxPerBeat : 0));
  const scrollBeats = useTimelineView(arrangementView, (s) => (range ? s.scrollBeats : 0));
  if (!range) return null;
  const box = rangeBox(range, rows, { pxPerBeat, scrollBeats });
  if (!box) return null;
  return (
    <>
      {(["start", "end"] as const).map((edge) => (
        <span
          key={edge}
          className={`eth-range-handle eth-range-handle--${edge}`}
          data-testid={`range-handle-${edge}`}
          style={{ left: edge === "start" ? box.x0 : box.x1, top: box.y0, height: box.y1 - box.y0 }}
        >
          <span className="eth-range-handle__knob" />
        </span>
      ))}
    </>
  );
}
