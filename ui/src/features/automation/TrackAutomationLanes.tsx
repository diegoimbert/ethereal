/**
 * The automation lanes of one track, as mounted under the track's row in the arrangement
 * (and in the detail view). Open/closed with `useAutomationToggle` (an icon button in the
 * track header). Open: a bar with a "show parameter" menu, then one row per shown
 * parameter: a header (`headerWidth` px) and the SVG lane. Closed: nothing. The master
 * track's lanes start with "Tempo" (`TEMPO_LANE_KEY`): the project's tempo map, edited like
 * in the ruler (`features/tempo`'s `TempoAutomationLane`).
 *
 * Its height is `automationHeight(state, track)` (see uiStore.ts), which the arrangement
 * feeds to its row layout.
 */

import clsx from "clsx";
import { memo, useMemo } from "react";
import type { AutomationLane, AutomationOwner, AutomationPoint, CurveShape, TrackId } from "@/generated";
import { Button, Select, type SelectOption } from "@/kit";
import { useProjectStore } from "@/state";
import { DEFAULT_GRID, itemSelection, useSelectedItems, type GridSetting, type ItemSelectionStore, type TimelineViewStore } from "@/timeline";
import { cmd, useTransport } from "@/transport";
import { PlayheadBpm, TempoAutomationLane } from "@/features/tempo";
import { setCurveCommand } from "./edit";
import { sendEdit } from "./gesture";
import { AutomationLaneView } from "./AutomationLaneView";
import { targetNameIsUserText, useTrackTargets, type TargetInfo } from "./params";
import { AUTOMATION_BAR_HEIGHT, automationHeight, laneHeightOf, laneUiKey, shownKeys, TEMPO_LANE_KEY, useAutomationUi } from "./uiStore";
import { defaultRange, FULL_RANGE } from "./valueAxis";
import { LANE_PAD } from "./geometry";
import { LaneResizeHandle, ValueScale } from "./ValueScale";
import { useTrackLanes } from "./toggle";
import { useLanesWithPoints } from "./clipEnvelopeTargets";
import { t as tr, useT, type MessageKey } from "@/i18n";
import { usePickerActiveOnly } from "./pickerPrefs";
import { useLaneAnimating, useShownLanes } from "./laneMotion";
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
  /** The lanes are drawn by the WebGL arranger (see `AutomationLaneView`'s `gl`). */
  gl?: boolean;
}

export const TrackAutomationLanes = memo(function TrackAutomationLanes(props: TrackAutomationLanesProps) {
  const { trackId } = props;
  const isOpen = useAutomationUi((s) => s.open.has(trackId));
  const animating = useLaneAnimating(trackId);
  const height = useAutomationUi((s) => automationHeight(s, trackId));
  // Closed (and not closing): an empty slot that subscribes to nothing else. The parameter
  // lists, descriptors and lanes are only computed for open tracks (
  // every mixer or device change used to rebuild them for all tracks).
  if (!isOpen && !animating) {
    return (
      <div
        className="eth-auto-track"
        style={{ height }}
        data-automation-track={trackId}
        onPointerDown={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
      />
    );
  }
  return <OpenTrackAutomationLanes {...props} isOpen={isOpen} animating={animating} height={height} />;
});

function OpenTrackAutomationLanes({
  trackId,
  view,
  headerWidth = 200,
  grid = DEFAULT_GRID,
  selection = itemSelection,
  gl = false,
  isOpen,
  animating,
  height,
}: TrackAutomationLanesProps & { isOpen: boolean; animating: boolean; height: number }) {
  const t = useT();
  const shownNow = useAutomationUi((s) => shownKeys(s, trackId));
  // While the height animates, lanes that went away stay mounted to fade/collapse out
  // (the arrangement row clips them), and new ones fade in. Mounted from closed.
  const shown = useShownLanes(trackId, isOpen, shownNow, true);
  const targets = useTrackTargets(trackId);
  const lanes = useTrackLanes(trackId);
  const trackName = useProjectStore((s) => s.project?.tracks[trackId]?.name ?? "");
  const isMaster = useProjectStore((s) => s.project?.tracks[trackId]?.kind === "Master");

  const withPoints = useLanesWithPoints(lanes);
  const [activeOnly, setActiveOnly] = usePickerActiveOnly();
  const hidden = targets.filter((t) => !shownNow.includes(t.key));
  const tempoHidden = isMaster && !shownNow.includes(TEMPO_LANE_KEY);
  const onShow = (key: string) => {
    if (key) useAutomationUi.getState().show(trackId, key);
  };

  return (
    <div
      className={clsx("eth-auto-track", animating && "eth-auto-track--animating", shown.closing && "eth-auto-track--closing")}
      // Mid-animation: fills the slot the arrangement gives it (the tweened height) and
      // clips the content, which keeps its natural height.
      style={{ height: animating ? undefined : height }}
      data-automation-track={trackId}
      // Keep lane and header gestures away from the arrangement's own handlers, except on the
      // bar's empty space (right of "+ Parameter…"): there a press is one on the track's empty
      // lane space (select the track, insert marker / playhead; drag: select clips or, with
      // ⇧, a time section).
      onPointerDown={(e) => {
        if (!(e.target as Element).classList?.contains("eth-auto-bar")) e.stopPropagation();
      }}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      {shown.open && (
        <div className={clsx("eth-auto-bar", shown.barEntering && "eth-auto-enter")} style={{ height: AUTOMATION_BAR_HEIGHT }}>
          <div className="eth-auto-bar__header" style={{ width: headerWidth }} aria-label={t("automation.bar.label", { name: trackName })}>
            {(hidden.length > 0 || tempoHidden) && (
              <Select
                size="sm"
                className="eth-auto-select"
                aria-label={t("automation.bar.show_parameter")}
                value=""
                placeholder={t("automation.bar.add_parameter")}
                options={tempoHidden ? [tempoOption(), ...targetOptions(hidden, lanes, withPoints)] : targetOptions(hidden, lanes, withPoints)}
                searchable
                activeFilter={{ value: activeOnly, onChange: setActiveOnly }}
                onChange={onShow}
              />
            )}
            <DrawModeToggle />
          </div>
        </div>
      )}
      {shown.keys.map((key) =>
        key === TEMPO_LANE_KEY ? (
          isMaster && (
            <TempoLaneRow
              key={key}
              className={clsx(shown.entering.has(key) && "eth-auto-enter", shown.leaving.has(key) && "eth-auto-row--leaving")}
              trackId={trackId}
              view={view}
              headerWidth={headerWidth}
            />
          )
        ) : (
          <LaneRow
            key={key}
            className={clsx(shown.entering.has(key) && "eth-auto-enter", shown.leaving.has(key) && "eth-auto-row--leaving")}
            trackId={trackId}
            targetKey={key}
            info={targets.find((t) => t.key === key)}
            targets={targets}
            shown={shownNow}
            lane={lanes.get(key) ?? null}
            view={view}
            headerWidth={headerWidth}
            grid={grid}
            selection={selection}
            gl={gl}
          />
        ),
      )}
    </div>
  );
}

/** The master's tempo lane in the "show parameter" menu. */
function tempoOption(): SelectOption<string> {
  return { value: TEMPO_LANE_KEY, label: tr("automation.tempo.name"), group: tr("automation.bar.project_group") };
}

/** Parameter choices grouped by device (a dot marks parameters that already have a lane). */
function targetOptions(
  targets: ReadonlyArray<TargetInfo>,
  lanes: ReadonlyMap<string, AutomationLane>,
  withPoints: ReadonlySet<string> = new Set(),
): SelectOption<string>[] {
  const groups = new Map<string, TargetInfo[]>();
  for (const t of targets) {
    const g = groups.get(t.group);
    if (g) g.push(t);
    else groups.set(t.group, [t]);
  }
  return [...groups].flatMap(([group, list]) =>
    list.map((t) => ({
      value: t.key,
      label: `${t.name}${lanes.has(t.key) ? " •" : ""}`,
      userText: targetNameIsUserText(t),
      group,
      active: withPoints.has(lanes.get(t.key)?.id ?? ""),
    })),
  );
}

/** The curve choices (`label`: a message key). */
const CURVE_OPTIONS: ReadonlyArray<{ value: CurveChoice; label: MessageKey }> = [
  { value: "Linear", label: "automation.curve.linear" },
  { value: "Step", label: "automation.curve.step" },
  { value: "Curve", label: "automation.curve.smooth" },
];

interface LaneRowProps {
  className?: string;
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
  gl: boolean;
}

type CurveChoice = "" | "Linear" | "Step" | "Curve";

function LaneRow({ className, trackId, targetKey: key, info, targets, shown, lane, view, headerWidth, grid, selection, gl }: LaneRowProps) {
  const transport = useTransport();
  const t = useT();
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

  const height = useAutomationUi((s) => laneHeightOf(s, trackId, key));
  const storedRange = useAutomationUi((s) => s.ranges[laneUiKey(trackId, key)]);
  const range = useMemo(() => storedRange ?? (info ? defaultRange(info.info, height - 2 * LANE_PAD) : FULL_RANGE), [storedRange, info, height]);

  const ui = useAutomationUi.getState;
  const onChangeTarget = (next: string) => ui().replace(trackId, key, next);
  const onCurve = (choice: CurveChoice) => {
    if (!choice) return;
    const ids = laneSelection.map((p) => p.id);
    const curve: CurveShape = choice === "Curve" ? { type: "Curve", tension: 0.5 } : choice === "Step" ? { type: "Step" } : { type: "Linear" };
    void sendEdit(transport, setCurveCommand(ids, curve));
  };

  return (
    <div className={clsx("eth-auto-row", className)} style={{ height }} data-target={key}>
      <div className="eth-auto-row__header" style={{ width: headerWidth }}>
        {info && <ValueScale info={info.info} range={range} height={height} name={info.name} onRange={(r) => ui().setRange(trackId, key, r)} />}
        <LaneResizeHandle trackId={trackId} targetKey={key} height={height} name={info?.name ?? t("automation.lane.unnamed")} />
        <div className="eth-auto-row__line">
          {info ? (
            <Select
              size="sm"
              className="eth-auto-select eth-auto-row__param"
              aria-label={t("automation.lane.parameter")}
              value={key}
              options={targetOptions(
                targets.filter((t) => t.key === key || !shown.includes(t.key)),
                new Map(),
              )}
              onChange={onChangeTarget}
            />
          ) : (
            <span className="eth-auto-row__missing">{t("automation.lane.unavailable")}</span>
          )}
          <button
            type="button"
            className="eth-auto-icon"
            aria-label={t("automation.lane.hide")}
            title={t("automation.lane.hide")}
            onClick={() => ui().hide(trackId, key)}
          >
            ×
          </button>
        </div>
        {lane && (
          <div className="eth-auto-row__line">
            <button
              type="button"
              className={clsx("eth-auto-icon", lane.enabled && "eth-auto-icon--on")}
              aria-pressed={lane.enabled}
              aria-label={t("automation.lane.enabled")}
              title={lane.enabled ? t("automation.lane.disable") : t("automation.lane.enable")}
              onClick={() => void sendEdit(transport, cmd("Automation", { type: "SetLaneEnabled", id: lane.id, enabled: !lane.enabled }))}
            >
              ⏻
            </button>
            <Select<CurveChoice>
              size="sm"
              className="eth-auto-select"
              aria-label={t("automation.lane.curve_label")}
              title={t("automation.lane.curve_title")}
              value={curveValue}
              placeholder={t("automation.lane.curve_placeholder")}
              disabled={laneSelection.length === 0}
              options={CURVE_OPTIONS.map((o) => ({ value: o.value, label: t(o.label) }))}
              onChange={onCurve}
            />
            <button
              type="button"
              className="eth-auto-icon"
              aria-label={t("automation.lane.delete")}
              title={t("automation.lane.delete_title")}
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
            height={height}
            range={range}
            grid={grid}
            selection={selection}
            label={t("automation.lane.label", { name: info.name })}
            gl={gl}
          />
        )}
      </div>
    </div>
  );
}

/**
 * The master's "Tempo" lane: a header ("Tempo", the BPM at the playhead, hide) and the
 * tempo map's lane (`TempoAutomationLane`), resizable like the other lanes.
 */
function TempoLaneRow({ className, trackId, view, headerWidth }: { className?: string; trackId: TrackId; view: TimelineViewStore; headerWidth: number }) {
  const height = useAutomationUi((s) => laneHeightOf(s, trackId, TEMPO_LANE_KEY));
  const t = useT();
  return (
    <div className={clsx("eth-auto-row", className)} style={{ height }} data-target={TEMPO_LANE_KEY} data-testid="tempo-automation-lane">
      <div className="eth-auto-row__header eth-auto-row__header--plain" style={{ width: headerWidth }}>
        <LaneResizeHandle trackId={trackId} targetKey={TEMPO_LANE_KEY} height={height} name={t("automation.tempo.name")} />
        <div className="eth-auto-row__line">
          <span className="eth-auto-row__name">{t("automation.tempo.name")}</span>
          <PlayheadBpm className="eth-auto-row__value" />
          <button
            type="button"
            className="eth-auto-icon"
            aria-label={t("automation.lane.hide")}
            title={t("automation.lane.hide")}
            onClick={() => useAutomationUi.getState().hide(trackId, TEMPO_LANE_KEY)}
          >
            ×
          </button>
        </div>
      </div>
      <div className="eth-auto-row__lane">
        <TempoAutomationLane view={view} height={height} />
      </div>
    </div>
  );
}

/** The bar's draw mode switch (`AutomationUiState.drawMode`, shared by every lane). */
function DrawModeToggle() {
  const t = useT();
  const on = useAutomationUi((s) => s.drawMode);
  return (
    <Button
      size="sm"
      active={on}
      aria-pressed={on}
      data-testid="automation-draw-mode"
      title={t("automation.bar.draw_title")}
      onClick={() => useAutomationUi.getState().setDrawMode(!on)}
    >
      {t("automation.bar.draw")}
    </Button>
  );
}
