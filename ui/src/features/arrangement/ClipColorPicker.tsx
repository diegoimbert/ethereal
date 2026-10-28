// "change color…" of the clip menu: the palette, opened where the menu was, for every
// clip it was opened on (one undo step).
import { Popover } from "@/kit";
import { ColorSwatches } from "@/features/colors";
import { useProjectStore } from "@/state";
import { useTransport } from "@/transport";
import { useT } from "@/i18n";
import { closeClipColorPicker, setClipsColor, useClipColorPicker } from "./clipColor";

/** The palette, mounted once by the arrangement. */
export function ClipColorPicker() {
  const open = useClipColorPicker((s) => s.open);
  // Mounted only while open (the transport is read then).
  return open ? <Palette open={open} /> : null;
}

function Palette({ open }: { open: NonNullable<ReturnType<typeof useClipColorPicker.getState>["open"]> }) {
  const t = useT();
  const transport = useTransport();
  const project = useProjectStore((s) => s.project);
  const clips = open && project ? open.clips.map((id) => project.clips[id]).filter((c) => c !== undefined) : [];
  if (!project || clips.length === 0) return null;
  const close = closeClipColorPicker;
  const first = clips[0]!;
  // The shared color (-1: they differ, no swatch checked); "use track color" shows the
  // track's color when they are all on one track.
  const value = clips.every((c) => c.color === first.color) ? first.color : -1;
  const oneTrack = clips.every((c) => c.track === first.track);
  const label = t("arrangement.clip_menu.color_named", { count: clips.length });
  return (
    <span className="eth-clip-color-anchor" style={{ position: "fixed", left: open.x, top: open.y, width: 0, height: 0 }}>
      <Popover open onOpenChange={(o) => !o && close()} placement="bottom-start" aria-label={label} trigger={() => <span />}>
        <div className="eth-color-palette">
          <ColorSwatches
            label={label}
            value={value}
            inherit={oneTrack ? (project.tracks[first.track]?.color ?? null) : null}
            onPick={(color) => {
              close();
              void setClipsColor(
                transport,
                clips.map((c) => c.id),
                color,
              );
            }}
          />
        </div>
      </Popover>
    </span>
  );
}
