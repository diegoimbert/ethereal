import { LocateFixed, MoreHorizontal, Plus } from "lucide-react";
import type { GridSetting } from "@/timeline";
import { useTimelineView } from "@/timeline";
import { Button, IconButton, Popover, Select } from "@/kit";
import { useReadOnly } from "@/state";
import { useT } from "@/i18n";
import { AddTrackMenu } from "./newTrack";
import { GRID_OPTIONS } from "./helpers";
import { arrangementView, useArrangementUi } from "./uiStore";

/** Below this header column width (px), Grid moves into the "More" menu. */
const COMPACT_BELOW = 230;

function gridId(grid: GridSetting): string {
  return GRID_OPTIONS.find((o) => JSON.stringify(o.grid) === JSON.stringify(grid))?.id ?? "adaptive-medium";
}

function GridSelect() {
  const t = useT();
  const grid = useArrangementUi((s) => s.grid);
  return (
    <span className="eth-arr-toolbar__grid">
      {t("arrangement.toolbar.grid")}
      <Select
        size="sm"
        aria-label={t("arrangement.toolbar.grid")}
        value={gridId(grid)}
        options={GRID_OPTIONS.map((o) => ({ value: o.id, label: t(o.label), short: o.short && t(o.short) }))}
        onChange={(id) => {
          const o = GRID_OPTIONS.find((x) => x.id === id);
          if (o) useArrangementUi.getState().setGrid(o.grid);
        }}
      />
    </span>
  );
}

/**
 * The arrangement's tools, in the corner above the track headers (the ruler's left): Add track,
 * the grid and Follow. A narrow header column keeps the grid in a "More" menu.
 */
export function Toolbar() {
  const compact = useArrangementUi((s) => s.headerWidth < COMPACT_BELOW);
  const follow = useTimelineView(arrangementView, (s) => s.followPlayhead);
  const readOnly = useReadOnly();
  const t = useT();

  return (
    <div className="eth-arr-toolbar" role="toolbar" aria-label={t("arrangement.toolbar.label")}>
      {!readOnly && (
        <AddTrackMenu
          trigger={(p) => (
            // Short in the toolbar (the lane's own button, under the tracks, says "add track").
            <Button
              size="sm"
              tone="ghost"
              className="eth-arr-toolbar__add"
              data-tour="add-track"
              aria-label={t("arrangement.new_track.add")}
              title={t("arrangement.toolbar.add_track_hint")}
              {...p}
            >
              <Plus aria-hidden /> {t("arrangement.toolbar.add")}
            </Button>
          )}
        />
      )}
      <span className="eth-arr-toolbar__spacer" />
      {!compact && <GridSelect />}
      <IconButton
        size="sm"
        tone="ghost"
        label={t("arrangement.toolbar.follow")}
        icon={<LocateFixed />}
        active={follow}
        onClick={() => arrangementView.getState().setFollowPlayhead(!follow)}
      />
      {compact && (
        <Popover
          placement="bottom-end"
          aria-label={t("arrangement.toolbar.more")}
          trigger={(p) => <IconButton size="sm" tone="ghost" label={t("arrangement.toolbar.more")} icon={<MoreHorizontal />} {...p} />}
        >
          <div className="eth-arr-toolbar__more">
            <GridSelect />
          </div>
        </Popover>
      )}
    </div>
  );
}
