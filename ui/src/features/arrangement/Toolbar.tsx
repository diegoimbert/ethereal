import type { GridSetting } from "@/timeline";
import { useTimelineView } from "@/timeline";
import { Button } from "@/kit";
import { useArrangement } from "./context";
import { runClipAction } from "./actions";
import { GRID_OPTIONS } from "./helpers";
import { arrangementView, useArrangementUi } from "./uiStore";

function gridId(grid: GridSetting): string {
  return GRID_OPTIONS.find((o) => JSON.stringify(o.grid) === JSON.stringify(grid))?.id ?? "adaptive-medium";
}

export function Toolbar() {
  const { transport } = useArrangement();
  const grid = useArrangementUi((s) => s.grid);
  const follow = useTimelineView(arrangementView, (s) => s.followPlayhead);
  const run = (a: Parameters<typeof runClipAction>[1]) => () => void runClipAction(transport, a);

  return (
    <div className="eth-arr-toolbar" role="toolbar" aria-label="Arrangement tools">
      <label className="eth-arr-toolbar__grid">
        Grid
        <select
          aria-label="Grid"
          value={gridId(grid)}
          onChange={(e) => {
            const o = GRID_OPTIONS.find((x) => x.id === e.target.value);
            if (o) useArrangementUi.getState().setGrid(o.grid);
          }}
        >
          {GRID_OPTIONS.map((o) => (
            <option key={o.id} value={o.id}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
      <Button
        size="sm"
        active={follow}
        title="Follow the playhead while playing"
        onClick={() => arrangementView.getState().setFollowPlayhead(!follow)}
      >
        Follow
      </Button>
      <span className="eth-arr-toolbar__sep" />
      <Button size="sm" onClick={run("split")} title="Split selected clips at the playhead (Ctrl/Cmd+E)">
        Split
      </Button>
      <Button size="sm" onClick={run("duplicate")} title="Duplicate selected clips (Ctrl/Cmd+D)">
        Duplicate
      </Button>
      <Button size="sm" onClick={run("loop")} title="Toggle looping of selected clips (Ctrl/Cmd+Shift+L)">
        Loop
      </Button>
      <Button size="sm" onClick={run("delete")} title="Delete selected clips (Delete)">
        Delete
      </Button>
      <span className="eth-arr-toolbar__sep" />
      <Button size="sm" aria-label="Zoom out" onClick={() => arrangementView.getState().zoomBy(1 / 1.5)}>
        −
      </Button>
      <Button size="sm" aria-label="Zoom in" onClick={() => arrangementView.getState().zoomBy(1.5)}>
        +
      </Button>
    </div>
  );
}
