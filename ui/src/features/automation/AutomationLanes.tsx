/**
 * Detail-view automation editor (the app shell's "Automation" tab): the selected track's
 * lanes with their own ruler and zoom/scroll. Above them, the shell's `clipLanes` slot: the
 * open MIDI clip's own controller lanes (`@/features/expression`'s
 * `EditedClipControllerLanes`), on the same timeline.
 */

import { useMemo, useRef, type ReactNode } from "react";
import { useClip, useEditedClipId, useSelectedTrackId, useTrack } from "@/state";
import { createTimelineViewStore, PlayheadLine, useTimelineWheel, type TimelineViewStore } from "@/timeline";
import { TempoRuler } from "@/features/tempo";
import { AutomationToggleButton } from "./AutomationToggleButton";
import { TrackAutomationLanes } from "./TrackAutomationLanes";
import { useT } from "@/i18n";
import "./automation.css";

const HEADER_WIDTH = 200;

export interface AutomationLanesProps {
  /** Rows drawn above the track's lanes, on the same view (the open clip's own lanes). */
  clipLanes?: (view: TimelineViewStore, headerWidth: number) => ReactNode;
}

/** Detail-view automation editor: the selected track's lanes with their own ruler. */
export function AutomationLanes({ clipLanes }: AutomationLanesProps = {}) {
  const t = useT();
  const edited = useClip(useEditedClipId());
  // No track selected: the open clip's.
  const trackId = useSelectedTrackId() ?? edited?.track ?? null;
  const track = useTrack(trackId);
  const view = useMemo(() => createTimelineViewStore({ pxPerBeat: 24 }), []);
  const bodyRef = useRef<HTMLDivElement>(null);
  useTimelineWheel(bodyRef, view, { originPx: HEADER_WIDTH, smoothScrollY: true });

  if (!trackId || !track) {
    return (
      <div className="eth-auto-detail eth-auto-detail--empty" data-feature="automation">
        {t("automation.detail.empty")}
      </div>
    );
  }
  return (
    <div className="eth-auto-detail" data-feature="automation">
      <div className="eth-auto-detail__ruler">
        <div className="eth-auto-detail__corner" style={{ width: HEADER_WIDTH }}>
          <span className="eth-auto-detail__name eth-user-text">{track.name}</span>
          <AutomationToggleButton trackId={trackId} trackName={track.name} />
        </div>
        <div className="eth-auto-detail__ruler-main">
          <TempoRuler view={view} />
        </div>
      </div>
      <div className="eth-auto-detail__body" ref={bodyRef}>
        {clipLanes?.(view, HEADER_WIDTH)}
        <TrackAutomationLanes trackId={trackId} view={view} headerWidth={HEADER_WIDTH} />
        <div className="eth-auto-detail__playhead" style={{ left: HEADER_WIDTH }}>
          <PlayheadLine view={view} />
        </div>
      </div>
    </div>
  );
}
