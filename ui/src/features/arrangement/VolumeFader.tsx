import { useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent } from "react";
import { formatDb } from "@/domain/paramScale";
import { setDragCursor } from "@/kit";
import { isReadOnly } from "@/state";
import { useT } from "@/i18n";
import { dbToFader, faderToDb } from "./faderLaw";
import { readoutBelow } from "./readoutPlacement";

export interface VolumeFaderProps {
  /** The level shown (dB; at or below -144: -inf). */
  db: number;
  /** Accessible name, e.g. "Keys volume". */
  label: string;
  /** A new level: during a drag, from an arrow / Home / End key, or a double-click (0 dB). */
  onChange(db: number, from: "drag" | "key" | "reset"): void;
  /** A drag starts / ends (one undo gesture). */
  onBegin(): void;
  onEnd(): void;
  /** Extra attributes on the slider (e.g. the MIDI-learn target). */
  attrs?: Record<string, string>;
  /** Right-click (the app's value menu: automation, set value, reset). */
  onContextMenu?: (e: MouseEvent<HTMLDivElement>) => void;
}

/**
 * The track header's compact horizontal volume fader (`.eth-arr-vol`), shared with rack
 * chains: drag left / right (Shift: fine) as one gesture, arrows step it, Home / End go to
 * the ends, double-click resets to 0 dB; the dB value shows while hovering or dragging.
 */
export function VolumeFader({ db, label, onChange, onBegin, onEnd, attrs, onContextMenu }: VolumeFaderProps) {
  const t = useT();
  const pos = dbToFader(db);
  const drag = useRef<{ x: number; pos: number; width: number } | null>(null);
  const [active, setActive] = useState(false);
  const [below, setBelow] = useState(false);
  const place = (el: HTMLDivElement) => setBelow(readoutBelow(el, el.querySelector(".eth-arr-vol__readout")));
  const set = (n: number, from: "drag" | "key" | "reset") => onChange(faderToDb(Math.min(1, Math.max(0, n))), from);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    // A shared project's viewer: shown, not moved.
    if (isReadOnly()) {
      e.stopPropagation();
      return;
    }
    e.stopPropagation(); // not a header / row press
    if (e.button !== 0) return;
    place(e.currentTarget);
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    e.currentTarget.focus({ preventScroll: true });
    drag.current = { x: e.clientX, pos, width: Math.max(1, e.currentTarget.clientWidth) };
    setActive(true);
    setDragCursor("ew-resize");
    onBegin();
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const k = e.shiftKey ? 0.1 : 1;
    set(d.pos + ((e.clientX - d.x) / d.width) * k, "drag");
  };
  const end = () => {
    if (!drag.current) return;
    drag.current = null;
    setActive(false);
    setDragCursor(null);
    onEnd();
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 0.01 : 0.05;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") set(pos + step, "key");
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") set(pos - step, "key");
    else if (e.key === "Home") set(0, "key");
    else if (e.key === "End") set(1, "key");
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div
      className={active ? "eth-arr-vol eth-arr-vol--active" : "eth-arr-vol"}
      role="slider"
      tabIndex={0}
      {...attrs}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={pos}
      aria-valuetext={formatDb(db)}
      data-readout={below ? "below" : undefined}
      title={t("arrangement.header.volume_title", { value: formatDb(db) })}
      onPointerEnter={(e) => place(e.currentTarget)}
      onContextMenu={onContextMenu}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => {
        e.stopPropagation();
        set(dbToFader(0), "reset");
      }}
      onKeyDown={onKeyDown}
    >
      <span className="eth-arr-vol__track">
        <span className="eth-arr-vol__fill" style={{ transform: `scaleX(${pos})` }} />
        <span className="eth-arr-vol__thumb" style={{ left: `${pos * 100}%` }} />
      </span>
      <span className="eth-arr-vol__readout" aria-hidden>
        {formatDb(db)}
      </span>
    </div>
  );
}
