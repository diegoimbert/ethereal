import { useEffect, useState } from "react";
import { useT } from "@/i18n";
import { sampleNoteName, sortedSet } from "./store";

const KEYS = Array.from({ length: 128 }, (_, k) => k);
const BLACK = new Set([1, 3, 6, 8, 10]);
const isBlack = (key: number) => BLACK.has(key % 12);
/** White keys left of `key` (its white-key column; a black key sits on the line after it). */
const whiteIndex = (key: number) => KEYS.slice(0, key).filter((k) => !isBlack(k)).length;
const WHITE_COUNT = KEYS.filter((k) => !isBlack(k)).length;

interface Drag {
  /** The key pressed first, and whether the drag adds (true) or removes keys. */
  anchor: number;
  add: boolean;
  /** The selection when the drag started. */
  base: ReadonlySet<number>;
}

/** `base` with every key from `a` to `b` added (or removed). */
function swept(base: ReadonlySet<number>, a: number, b: number, add: boolean): number[] {
  const out = new Set(base);
  for (let k = Math.min(a, b); k <= Math.max(a, b); k++) {
    if (add) out.add(k);
    else out.delete(k);
  }
  return sortedSet(out);
}

/**
 * The full MIDI keyboard (C-2 to G8) with the sampled keys lit. A click on a key adds or
 * removes it; a drag sweeps every key between the pressed one and the pointer, adding them
 * when the pressed key was off and removing them when it was on.
 */
export function SampleKeyboard({ keys, onChange }: { keys: readonly number[]; onChange: (keys: number[]) => void }) {
  const t = useT();
  const selected = new Set(keys);
  const [drag, setDrag] = useState<Drag | null>(null);
  useEffect(() => {
    if (!drag) return;
    const end = () => setDrag(null);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
    return () => {
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
    };
  }, [drag]);

  const press = (key: number) => {
    const d = { anchor: key, add: !selected.has(key), base: selected };
    setDrag(d);
    onChange(swept(d.base, key, key, d.add));
  };
  const enter = (key: number) => {
    if (drag) onChange(swept(drag.base, drag.anchor, key, drag.add));
  };

  return (
    <div className="eth-sample-keys" role="group" aria-label={t("sampler.convert.keyboard")} data-dragging={drag ? "" : undefined}>
      {KEYS.map((key) => {
        const black = isBlack(key);
        // A white key's left edge, or the line a black key is centred on.
        const left = (whiteIndex(key) / WHITE_COUNT) * 100;
        return (
          <div
            key={key}
            role="checkbox"
            aria-checked={selected.has(key)}
            aria-label={sampleNoteName(key)}
            title={sampleNoteName(key)}
            data-key={key}
            className={black ? "eth-sample-keys__key eth-sample-keys__key--black" : "eth-sample-keys__key"}
            style={{ left: `${left}%` }}
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              // No text selection or focus change while sweeping.
              e.preventDefault();
              press(key);
            }}
            onPointerEnter={() => enter(key)}
          >
            {key % 12 === 0 && <span className="eth-sample-keys__label">{sampleNoteName(key)}</span>}
          </div>
        );
      })}
    </div>
  );
}
