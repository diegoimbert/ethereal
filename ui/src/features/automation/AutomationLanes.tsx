/**
 * Detail-view automation editor (the app shell's "Automation" tab): the selected track's
 * lanes with their own ruler and zoom/scroll.
 */

import { useMemo, useRef } from "react";
import { useSelectedTrackId, useTrack } from "@/state";
import { createTimelineViewStore, PlayheadLine, Ruler, useTimelineWheel } from "@/timeline";
import { TrackAutomationLanes } from "./TrackAutomationLanes";
import "./automation.css";

const HEADER_WIDTH = 200;

/** Detail-view automation editor: the selected track's lanes with their own ruler. */
export function AutomationLanes() {
  const trackId = useSelectedTrackId();
  const track = useTrack(trackId);
  const view = useMemo(() => createTimelineViewStore({ pxPerBeat: 24 }), []);
  const bodyRef = useRef<HTMLDivElement>(null);
  useTimelineWheel(bodyRef, view, { originPx: HEADER_WIDTH });

  if (!trackId || !track) {
    return (
      <div className="eth-auto-detail eth-auto-detail--empty" data-feature="automation">
        Select a track to edit its automation.
      </div>
    );
  }
  return (
    <div className="eth-auto-detail" data-feature="automation">
      <div className="eth-auto-detail__ruler">
        <div className="eth-auto-detail__corner" style={{ width: HEADER_WIDTH }}>
          {track.name}
        </div>
        <div className="eth-auto-detail__ruler-main">
          <Ruler view={view} />
        </div>
      </div>
      <div className="eth-auto-detail__body" ref={bodyRef}>
        <TrackAutomationLanes trackId={trackId} view={view} headerWidth={HEADER_WIDTH} />
        <div className="eth-auto-detail__playhead" style={{ left: HEADER_WIDTH }}>
          <PlayheadLine view={view} />
        </div>
      </div>
    </div>
  );
}
