import { AudioLines, Piano, Plus, X } from "lucide-react";
import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import { Button, IconButton, setDragCursor } from "@/kit";
import { addTrack, selectTrackEntity, type NewTrackKind } from "./actions";
import { useArrangement } from "./context";
import { DRAFT_TRACK_ID, type Row } from "./layout";
import { trackDropTarget } from "./trackDrag";
import { useArrangementUi } from "./uiStore";

const DRAG_THRESHOLD_PX = 4;
const INDENT_PX = 12;

/**
 * "+ New track": a click adds a draft track after the last regular track; dragging the
 * button between rows (or onto a group) puts the draft there. The draft row then asks
 * for the type in place (see `DraftRow`).
 */
export function NewTrackButton() {
  const ctx = useArrangement();
  const dragged = useRef(false);

  const onPointerDown = (e: PointerEvent<HTMLButtonElement>) => {
    if (e.button !== 0) return;
    dragged.current = false;
    const startX = e.clientX;
    const startY = e.clientY;
    const ui = useArrangementUi.getState;
    let target: ReturnType<typeof trackDropTarget> = null;

    const move = (ev: globalThis.PointerEvent) => {
      if (!dragged.current && Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD_PX) return;
      if (!dragged.current) setDragCursor("grabbing");
      dragged.current = true;
      const box = ctx.contentRef.current?.getBoundingClientRect();
      const inside = box && ev.clientX >= box.left && ev.clientX <= box.right && ev.clientY >= box.top && ev.clientY <= box.bottom;
      target = inside ? trackDropTarget(ctx.rowsRef.current, ev.clientY - box.top, DRAFT_TRACK_ID) : null;
      ui().setTrackDrag(target ? { track: DRAFT_TRACK_ID, y: target.y, into: target.into } : null);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      setDragCursor(null);
      ui().setTrackDrag(null);
      if (dragged.current && target) ui().setDraftTrack({ parent: target.parent, before: target.before });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  return (
    <Button
      size="sm"
      className="eth-arr-toolbar__new"
      title="Add a track (or drag it between tracks)"
      onPointerDown={onPointerDown}
      onClick={() => {
        // A drag ends with a click too: the drop already placed the draft.
        if (dragged.current) {
          dragged.current = false;
          return;
        }
        useArrangementUi.getState().setDraftTrack({ parent: null, before: null });
      }}
    >
      <Plus aria-hidden /> New track
    </Button>
  );
}

/** The draft track's row: asks whether it is a MIDI or an audio track (M / A, Esc cancels). */
export function DraftRow({ row }: { row: Row }) {
  const { transport } = useArrangement();
  const draft = row.draft!;
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const cancel = () => useArrangementUi.getState().setDraftTrack(null);
  const create = (kind: NewTrackKind) => {
    cancel();
    void addTrack(transport, kind, draft).then((id) => selectTrackEntity(id));
  };
  const onKeyDown = (e: KeyboardEvent) => {
    const k = e.key.toLowerCase();
    if (k === "m" || k === "a" || k === "escape") {
      e.preventDefault();
      e.stopPropagation();
      if (k === "escape") cancel();
      else create(k === "m" ? "Midi" : "Audio");
    }
  };

  return (
    <div className="eth-arr-row eth-arr-row--draft" style={{ height: row.height }} data-track={DRAFT_TRACK_ID} onKeyDown={onKeyDown}>
      <div className="eth-arr-row__main" style={{ height: row.laneHeight }}>
        <div
          className="eth-arr-header eth-arr-header--draft"
          style={{ width: headerWidth, paddingLeft: 14 + row.depth * INDENT_PX, ["--eth-track-depth" as string]: row.depth }}
          role="group"
          aria-label="New track"
          onPointerDown={(e) => e.stopPropagation()}
        >
          <span className="eth-arr-header__name">New track</span>
          <Button size="sm" autoFocus aria-label="Create MIDI track" title="MIDI track (M)" onClick={() => create("Midi")}>
            <Piano aria-hidden /> MIDI
          </Button>
          <Button size="sm" aria-label="Create audio track" title="Audio track (A)" onClick={() => create("Audio")}>
            <AudioLines aria-hidden /> Audio
          </Button>
          <IconButton size="sm" tone="ghost" label="Cancel new track" icon={<X />} onClick={cancel} />
        </div>
        <div className="eth-arr-lane eth-arr-lane--draft">MIDI or audio? Press M or A (Esc to cancel)</div>
      </div>
    </div>
  );
}
