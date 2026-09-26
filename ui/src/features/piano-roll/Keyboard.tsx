/** Keyboard gutter: one key per pitch row. Clicking a key selects that pitch's notes. */

import { memo } from "react";
import clsx from "clsx";
import type { Note } from "@/generated";
import { itemSelection, selectModeFromEvent } from "@/timeline";
import { isBlackKey, KEYBOARD_WIDTH, pitchName, pitchToY, PITCHES } from "./geometry";

export interface KeyboardProps {
  keyH: number;
  /** Notes of the clip (for click-to-select). */
  notes: ReadonlyArray<Note>;
}

export function Keyboard({ keyH, notes }: KeyboardProps) {
  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    const pitch = Number((e.target as HTMLElement).dataset.pitch);
    if (e.button !== 0 || Number.isNaN(pitch)) return;
    const mode = selectModeFromEvent(e);
    const ids = notes.filter((n) => n.pitch === pitch).map((n) => n.id);
    itemSelection.getState().select("note", ids, mode === "remove" ? "replace" : mode);
  };
  return (
    <div
      className="eth-pr-keys"
      style={{ width: KEYBOARD_WIDTH, height: PITCHES * keyH }}
      onPointerDown={onPointerDown}
      data-testid="piano-roll-keys"
    >
      <Keys keyH={keyH} />
    </div>
  );
}

const Keys = memo(function Keys({ keyH }: { keyH: number }) {
  const keys = [];
  for (let p = PITCHES - 1; p >= 0; p--) {
    const black = isBlackKey(p);
    keys.push(
      <div
        key={p}
        data-pitch={p}
        className={clsx("eth-pr-key", black ? "eth-pr-key--black" : "eth-pr-key--white", p % 12 === 0 && "eth-pr-key--c")}
        style={{ top: pitchToY(p, keyH), height: keyH }}
        title={pitchName(p)}
      >
        {p % 12 === 0 && keyH >= 9 ? pitchName(p) : null}
      </div>,
    );
  }
  return <>{keys}</>;
});
