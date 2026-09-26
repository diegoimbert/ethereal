/**
 * The automation lanes of one track, as mounted under the track's row in the arrangement
 * (and in the detail view). A bar with the open/close toggle and a "show parameter" menu,
 * then one row per shown parameter: a header (`headerWidth` px) and the SVG lane.
 *
 * Its height is `automationHeight(state, track)` (see uiStore.ts), which the arrangement
 * feeds to its row layout.
 */

import clsx from "clsx";
import { useMemo, type ChangeEvent } from "react";
import type { AutomationLane, AutomationOwner, AutomationPoint, CurveShape, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { DEFAULT_GRID, itemSelection, useSelectedItems, type GridSetting, type ItemSelectionStore, type TimelineViewStore } from "@/timeline";
import { cmd, useTransport } from "@/transport";
import { setCurveCommand } from "./edit";
import { sendEdit } from "./gesture";
import { AutomationLaneView } from "./AutomationLaneView";
import { targetKey, useTrackTargets, type TargetInfo } from "./params";
import { AUTOMATION_BAR_HEIGHT, LANE_HEIGHT, automationHeight, shownKeys, useAutomationUi } from "./uiStore";
import "./automation.css";

export interface TrackAutomationLanesProps {
  trackId: TrackId;
  /** Horizontal viewport shared with the clip lanes (the arrangement's view store). */
  view: TimelineViewStore;
  /** Width of the header column, left of the lanes (default 200, the arrangement's). */
  headerWidth?: number;
  /** Grid used to snap point times (default: adaptive). */
  grid?: GridSetting;
  /** Point selection store (default: the app-wide `itemSelection`). */
  selection?: ItemSelectionStore;
}

const EMPTY_LANES: Readonly<Record<string, AutomationLane>> = {};

/** Arrangement lanes of a track, by target key. */
function useTrackLanes(track: TrackId): ReadonlyMap<string, AutomationLane> {
  const lanes = useProjectStore((s) => s.project?.automation_lanes ?? EMPTY_LANES);
  return useMemo(() => {
    const m = new Map<string, AutomationLane>();
    for (const l of Object.values(lanes)) if (l.owner.type === "Track" && l.owner.track === track) m.set(targetKey(l.target), l);
    return m;
  }, [lanes, track]);
}

export function TrackAutomationLanes({
  trackId,
  view,
  headerWidth = 200,
  grid = DEFAULT_GRID,
  selection = itemSelection,
}: TrackAutomationLanesProps) {
  const open = useAutomationUi((s) => s.open.has(trackId));
  const keys = useAutomationUi((s) => shownKeys(s, trackId));
  const height = useAutomationUi((s) => automationHeight(s, trackId));
  const targets = useTrackTargets(trackId);
  const lanes = useTrackLanes(trackId);
  const trackName = useProjectStore((s) => s.project?.tracks[trackId]?.name ?? "");

  const toggle = () => {
    const initial = lanes.size > 0 ? [...lanes.keys()] : [targetKey({ type: "TrackVolume", track: trackId })];
    useAutomationUi.getState().setOpen(trackId, !open, initial);
  };

  const hidden = targets.filter((t) => !keys.includes(t.key));
  const onShow = (e: ChangeEvent<HTMLSelectElement>) => {
    if (e.target.value) useAutomationUi.getState().show(trackId, e.target.value);
  };

  return (
    <div
      className="eth-auto-track"
      style={{ height }}
      data-automation-track={trackId}
      // Keep lane and header gestures away from the arrangement's own handlers.
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      <div className="eth-auto-bar" style={{ height: AUTOMATION_BAR_HEIGHT }}>
        <div className="eth-auto-bar__header" style={{ width: headerWidth }}>
          <button
            type="button"
            className="eth-auto-bar__toggle"
            aria-expanded={open}
            aria-label={`${open ? "Hide" : "Show"} automation of ${trackName}`}
            onClick={toggle}
          >
            {open ? "▾" : "▸"} Automation{lanes.size > 0 ? ` (${lanes.size})` : ""}
          </button>
          {open && hidden.length > 0 && (
            <select className="eth-auto-select" aria-label="Show parameter" value="" onChange={onShow}>
              <option value="">+ Parameter…</option>
              <TargetOptions targets={hidden} lanes={lanes} />
            </select>
          )}
        </div>
      </div>
      {keys.map((key) => (
        <LaneRow
          key={key}
          trackId={trackId}
          targetKey={key}
          info={targets.find((t) => t.key === key)}
          targets={targets}
          shown={keys}
          lane={lanes.get(key) ?? null}
          view={view}
          headerWidth={headerWidth}
          grid={grid}
          selection={selection}
        />
      ))}
    </div>
  );
}

function TargetOptions({ targets, lanes }: { targets: ReadonlyArray<TargetInfo>; lanes: ReadonlyMap<string, AutomationLane> }) {
  const groups = new Map<string, TargetInfo[]>();
  for (const t of targets) {
    const g = groups.get(t.group);
    if (g) g.push(t);
    else groups.set(t.group, [t]);
  }
  return (
    <>
      {[...groups].map(([group, list]) => (
        <optgroup key={group} label={group}>
          {list.map((t) => (
            <option key={t.key} value={t.key}>
              {t.name}
              {lanes.has(t.key) ? " •" : ""}
            </option>
          ))}
        </optgroup>
      ))}
    </>
  );
}

interface LaneRowProps {
  trackId: TrackId;
  targetKey: string;
  info: TargetInfo | undefined;
  targets: ReadonlyArray<TargetInfo>;
  shown: ReadonlyArray<string>;
  lane: AutomationLane | null;
  view: TimelineViewStore;
  headerWidth: number;
  grid: GridSetting;
  selection: ItemSelectionStore;
}

type CurveChoice = "" | "Linear" | "Step" | "Curve";

function LaneRow({ trackId, targetKey: key, info, targets, shown, lane, view, headerWidth, grid, selection }: LaneRowProps) {
  const transport = useTransport();
  const owner: AutomationOwner = useMemo(() => ({ type: "Track", track: trackId }), [trackId]);
  const selected = useSelectedItems("automationPoint", selection);
  const points = useProjectStore((s) => s.project?.automation_points);
  const laneSelection = useMemo(
    () => (lane && points ? [...selected].map((id) => points[id]).filter((p): p is AutomationPoint => p !== undefined && p.lane === lane.id) : []),
    [selected, points, lane],
  );
  const curveValue: CurveChoice = useMemo(() => {
    const types = new Set(laneSelection.map((p) => p.curve.type));
    return types.size === 1 ? ([...types][0] as CurveChoice) : "";
  }, [laneSelection]);

  const ui = useAutomationUi.getState;
  const onChangeTarget = (e: ChangeEvent<HTMLSelectElement>) => ui().replace(trackId, key, e.target.value);
  const onCurve = (e: ChangeEvent<HTMLSelectElement>) => {
    const choice = e.target.value as CurveChoice;
    if (!choice) return;
    const ids = laneSelection.map((p) => p.id);
    const curve: CurveShape =
      choice === "Curve" ? { type: "Curve", tension: 0.5 } : choice === "Step" ? { type: "Step" } : { type: "Linear" };
    void sendEdit(transport, setCurveCommand(ids, curve));
  };

  return (
    <div className="eth-auto-row" style={{ height: LANE_HEIGHT }} data-target={key}>
      <div className="eth-auto-row__header" style={{ width: headerWidth }}>
        <div className="eth-auto-row__line">
          {info ? (
            <select className="eth-auto-select eth-auto-row__param" aria-label="Automated parameter" value={key} onChange={onChangeTarget}>
              <TargetOptions targets={targets.filter((t) => t.key === key || !shown.includes(t.key))} lanes={new Map()} />
            </select>
          ) : (
            <span className="eth-auto-row__missing">Unavailable parameter</span>
          )}
          <button type="button" className="eth-auto-icon" aria-label="Hide lane" title="Hide lane" onClick={() => ui().hide(trackId, key)}>
            ×
          </button>
        </div>
        {lane && (
          <div className="eth-auto-row__line">
            <button
              type="button"
              className={clsx("eth-auto-icon", lane.enabled && "eth-auto-icon--on")}
              aria-pressed={lane.enabled}
              aria-label="Lane enabled"
              title={lane.enabled ? "Disable automation" : "Enable automation"}
              onClick={() => void sendEdit(transport, cmd("Automation", { type: "SetLaneEnabled", id: lane.id, enabled: !lane.enabled }))}
            >
              ⏻
            </button>
            <select
              className="eth-auto-select"
              aria-label="Curve of selected points"
              title="Curve of the segments after the selected points (alt-drag a segment to bend it)"
              value={curveValue}
              disabled={laneSelection.length === 0}
              onChange={onCurve}
            >
              <option value="">Curve…</option>
              <option value="Linear">Linear</option>
              <option value="Step">Step</option>
              <option value="Curve">Curve</option>
            </select>
            <button
              type="button"
              className="eth-auto-icon"
              aria-label="Delete lane"
              title="Delete lane and its points"
              onClick={() => void sendEdit(transport, cmd("Automation", { type: "DeleteLane", id: lane.id }))}
            >
              ⌫
            </button>
          </div>
        )}
      </div>
      <div className="eth-auto-row__lane">
        {info && (
          <AutomationLaneView
            lane={lane}
            owner={owner}
            target={info.target}
            info={info.info}
            view={view}
            height={LANE_HEIGHT}
            grid={grid}
            selection={selection}
            label={`${info.name} automation`}
          />
        )}
      </div>
    </div>
  );
}
