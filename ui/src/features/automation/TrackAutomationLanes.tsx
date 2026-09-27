/**
 * The automation lanes of one track, as mounted under the track's row in the arrangement
 * (and in the detail view). Open/closed with `useAutomationToggle` (an icon button in the
 * track header). Open: a bar with a "show parameter" menu, then one row per shown
 * parameter: a header (`headerWidth` px) and the SVG lane. Closed: nothing.
 *
 * Its height is `automationHeight(state, track)` (see uiStore.ts), which the arrangement
 * feeds to its row layout.
 */

import clsx from "clsx";
import { useMemo } from "react";
import type { AutomationLane, AutomationOwner, AutomationPoint, CurveShape, TrackId } from "@/generated";
import { Select, type SelectOption } from "@/kit";
import { useProjectStore } from "@/state";
import { DEFAULT_GRID, itemSelection, useSelectedItems, type GridSetting, type ItemSelectionStore, type TimelineViewStore } from "@/timeline";
import { cmd, useTransport } from "@/transport";
import { setCurveCommand } from "./edit";
import { sendEdit } from "./gesture";
import { AutomationLaneView } from "./AutomationLaneView";
import { useTrackTargets, type TargetInfo } from "./params";
import { AUTOMATION_BAR_HEIGHT, LANE_HEIGHT, automationHeight, shownKeys, useAutomationUi } from "./uiStore";
import { useTrackLanes } from "./toggle";
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

  const hidden = targets.filter((t) => !keys.includes(t.key));
  const onShow = (key: string) => {
    if (key) useAutomationUi.getState().show(trackId, key);
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
      {open && (
      <div className="eth-auto-bar" style={{ height: AUTOMATION_BAR_HEIGHT }}>
        <div className="eth-auto-bar__header" style={{ width: headerWidth }} aria-label={`Automation of ${trackName}`}>
          {hidden.length > 0 && (
            <Select
              size="sm"
              className="eth-auto-select"
              aria-label="Show parameter"
              value=""
              placeholder="+ Parameter…"
              options={targetOptions(hidden, lanes)}
              onChange={onShow}
            />
          )}
        </div>
      </div>
      )}
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

/** Parameter choices grouped by device (a dot marks parameters that already have a lane). */
function targetOptions(
  targets: ReadonlyArray<TargetInfo>,
  lanes: ReadonlyMap<string, AutomationLane>,
): SelectOption<string>[] {
  const groups = new Map<string, TargetInfo[]>();
  for (const t of targets) {
    const g = groups.get(t.group);
    if (g) g.push(t);
    else groups.set(t.group, [t]);
  }
  return [...groups].flatMap(([group, list]) =>
    list.map((t) => ({ value: t.key, label: `${t.name}${lanes.has(t.key) ? " •" : ""}`, group })),
  );
}

const CURVE_OPTIONS: ReadonlyArray<SelectOption<CurveChoice>> = [
  { value: "Linear", label: "Linear" },
  { value: "Step", label: "Step" },
  { value: "Curve", label: "Curve" },
];

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
  const onChangeTarget = (next: string) => ui().replace(trackId, key, next);
  const onCurve = (choice: CurveChoice) => {
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
            <Select
              size="sm"
              className="eth-auto-select eth-auto-row__param"
              aria-label="Automated parameter"
              value={key}
              options={targetOptions(
                targets.filter((t) => t.key === key || !shown.includes(t.key)),
                new Map(),
              )}
              onChange={onChangeTarget}
            />
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
            <Select<CurveChoice>
              size="sm"
              className="eth-auto-select"
              aria-label="Curve of selected points"
              title="Curve of the segments after the selected points (alt-drag a segment to bend it)"
              value={curveValue}
              placeholder="Curve…"
              disabled={laneSelection.length === 0}
              options={CURVE_OPTIONS}
              onChange={onCurve}
            />
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
