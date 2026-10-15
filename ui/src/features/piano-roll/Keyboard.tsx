/**
 * Keyboard gutter: one key per pitch row. Clicking a key selects that pitch's notes and
 * plays the pitch on the clip's track while held (note preview, `notePreview.ts`).
 */

import { memo, useState } from "react";
import clsx from "clsx";
import { scaleTone } from "@/domain/scales";
import type { MusicalScale, Note, TrackId } from "@/generated";
import { itemSelection, selectModeFromEvent } from "@/timeline";
import { useTransport } from "@/transport";
import { playPreview } from "./notePreview";
import { isBlackKey, KEYBOARD_WIDTH, pitchName, pitchToY, yToPitch } from "./geometry";

export interface KeyboardProps {
  keyH: number;
  rows: readonly number[];
  scale: MusicalScale;
  highlight: boolean;
  /** Notes of the clip (for click-to-select). */
  notes: ReadonlyArray<Note>;
  /** The clip's track (previewed pitches play on its instrument). */
  track?: TrackId;
}

/** Velocity of a key press preview (the default of a drawn note). */
const KEY_VELOCITY = 100 / 127;

export function Keyboard({ keyH, notes, rows, scale, highlight, track }: KeyboardProps) {
  const transport = useTransport();
  /** The key held down (drawn pressed until release: a press is felt, not just heard). */
  const [pressed, setPressed] = useState<number | null>(null);
  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    // A black key covers only the left of its row: right of it, the pitch is the row under
    // the pointer.
    const key = (e.target as HTMLElement).dataset.pitch;
    const pitch = key !== undefined ? Number(key) : yToPitch(e.clientY - e.currentTarget.getBoundingClientRect().top, keyH, rows);
    if (track) playPreview(transport, track, pitch, KEY_VELOCITY);
    setPressed(pitch);
    const release = () => {
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", release);
      setPressed(null);
    };
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
    const mode = selectModeFromEvent(e);
    const ids = notes.filter((n) => n.pitch === pitch).map((n) => n.id);
    itemSelection.getState().select("note", ids, mode === "remove" ? "replace" : mode);
  };
  return (
    <div className="eth-pr-keys" style={{ width: KEYBOARD_WIDTH, height: rows.length * keyH }} onPointerDown={onPointerDown} data-testid="piano-roll-keys">
      <Keys keyH={keyH} rows={rows} scale={scale} highlight={highlight} pressed={pressed} />
    </div>
  );
}

const Keys = memo(function Keys({ keyH, rows, scale, highlight, pressed }: Omit<KeyboardProps, "notes" | "track"> & { pressed: number | null }) {
  const keys = [];
  for (const p of rows) {
    const tone = scaleTone(p, scale, highlight);
    const black = isBlackKey(p);
    keys.push(
      <div
        key={p}
        data-pitch={p}
        data-scale-tone={tone}
        className={clsx(
          "eth-pr-key",
          "eth-user-text",
          black ? "eth-pr-key--black" : "eth-pr-key--white",
          p % 12 === 0 && "eth-pr-key--c",
          p === pressed && "eth-pr-key--pressed",
        )}
        style={{ top: pitchToY(p, keyH, rows), height: keyH }}
        title={pitchName(p)}
      >
        {(p % 12 === 0 || tone === "root") && keyH >= 9 ? pitchName(p) : null}
      </div>,
    );
  }
  return <>{keys}</>;
});
