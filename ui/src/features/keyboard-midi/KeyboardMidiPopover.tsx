/**
 * The computer MIDI keyboard's indicator (lowest note, velocity) as a button opening a
 * popover to set both with the mouse: a mini keyboard strip (the keys the computer keyboard
 * plays are lit; clicking a key moves the octave there) with octave down / up buttons, and a
 * velocity slider with the C/V stops as presets.
 *
 * It edits the same store the Z/X and C/V keys do (`keyboard.ts`), so both stay in sync while
 * it is open. The slider is an `<input>`: while it has focus it owns its keys (arrows step,
 * letters play nothing), like every text field; on the buttons the note keys keep playing.
 */
import { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Popover } from "@/kit";
import { useT } from "@/i18n";
import { setOctave, setVelocity, useKeyboardMidi } from "./keyboard";
import { MAX_OCTAVE, MAX_VELOCITY, MIN_OCTAVE, MIN_VELOCITY, NOTE_KEYS, noteName, octaveBase, STRIP_OCTAVES, stripStart, velocityStops } from "./notes";

const BLACK = new Set([1, 3, 6, 8, 10]);
const SPAN = Math.max(...Object.values(NOTE_KEYS));

export function KeyboardMidiPopover({ low, velocity, playing }: { low: string; velocity: number; playing: boolean }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      aria-label={t("keyboard_midi.settings")}
      placement="bottom-start"
      className="eth-kbmidi-pop"
      trigger={(props) => (
        <button
          {...props}
          type="button"
          className="eth-kbmidi__status"
          data-testid="keyboard-midi-status"
          data-playing={playing || undefined}
          aria-label={t("keyboard_midi.status", { low, velocity })}
          title={t("keyboard_midi.status_title")}
        >
          <span className="eth-kbmidi__octave eth-user-text">{low}</span>
          <span className="eth-kbmidi__velocity">
            <span className="eth-kbmidi__unit" aria-hidden>
              {t("keyboard_midi.vel")}
            </span>
            {velocity}
          </span>
        </button>
      )}
    >
      <KeyboardMidiSettings />
    </Popover>
  );
}

function KeyboardMidiSettings() {
  const t = useT();
  const octave = useKeyboardMidi((s) => s.octave);
  const velocity = useKeyboardMidi((s) => s.velocity);
  const first = useRef<HTMLButtonElement>(null);
  // Keyboard users land inside the popover (Escape returns to the indicator).
  useEffect(() => first.current?.focus({ preventScroll: true }), []);

  const base = octaveBase(octave);
  const start = octaveBase(stripStart(octave));
  const end = Math.min(127, start + 12 * STRIP_OCTAVES - 1);
  const keys: number[] = [];
  for (let n = start; n <= end; n++) keys.push(n);
  const whites = keys.filter((n) => !BLACK.has(n % 12));
  const lowName = noteName(base);
  const highName = noteName(Math.min(127, base + SPAN));

  return (
    <div className="eth-kbmidi-pop__body" data-feature="keyboard-midi-settings">
      <section className="eth-kbmidi-pop__section">
        <div className="eth-kbmidi-pop__row">
          <span className="eth-kbmidi-pop__label">{t("keyboard_midi.octave")}</span>
          <span className="eth-kbmidi-pop__value eth-user-text" data-testid="keyboard-midi-range">
            {lowName}–{highName}
          </span>
          <span className="eth-kbmidi-pop__stepper">
            <button
              ref={first}
              type="button"
              className="eth-kbmidi-pop__step"
              aria-label={t("keyboard_midi.octave_down")}
              title={t("keyboard_midi.octave_down_title", { key: "Z" })}
              disabled={octave <= MIN_OCTAVE}
              onClick={() => setOctave(octave - 1)}
            >
              <ChevronLeft aria-hidden />
            </button>
            <button
              type="button"
              className="eth-kbmidi-pop__step"
              aria-label={t("keyboard_midi.octave_up")}
              title={t("keyboard_midi.octave_up_title", { key: "X" })}
              disabled={octave >= MAX_OCTAVE}
              onClick={() => setOctave(octave + 1)}
            >
              <ChevronRight aria-hidden />
            </button>
          </span>
        </div>
        {/* Pointer shortcut only (the buttons above are the keyboard way): not in the tab order. */}
        <div className="eth-kbmidi-pop__strip" data-testid="keyboard-midi-strip" title={t("keyboard_midi.strip_title")} aria-hidden>
          {whites.map((n) => {
            const played = n >= base && n <= base + SPAN;
            return (
              // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- a pointer shortcut in an aria-hidden strip (the octave buttons are the keyboard way)
              <span
                key={n}
                className="eth-kbmidi-pop__white"
                data-note={n}
                data-played={played || undefined}
                data-root={n === base || undefined}
                onClick={() => setOctave(Math.floor(n / 12) - 2)}
              >
                {n % 12 === 0 && <span className="eth-kbmidi-pop__c eth-user-text">{noteName(n)}</span>}
              </span>
            );
          })}
          {keys
            .filter((n) => BLACK.has(n % 12))
            .map((n) => {
              const played = n >= base && n <= base + SPAN;
              // Centered on the line between its two white neighbours.
              const left = (whites.findIndex((w) => w > n) / whites.length) * 100;
              return (
                // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- a pointer shortcut in an aria-hidden strip (the octave buttons are the keyboard way)
                <span
                  key={n}
                  className="eth-kbmidi-pop__black"
                  data-note={n}
                  data-played={played || undefined}
                  style={{ left: `${left}%`, width: `${(100 / whites.length) * 0.6}%` }}
                  onClick={() => setOctave(Math.floor(n / 12) - 2)}
                />
              );
            })}
        </div>
      </section>
      <section className="eth-kbmidi-pop__section">
        <label className="eth-kbmidi-pop__row">
          <span className="eth-kbmidi-pop__label">{t("keyboard_midi.velocity")}</span>
          <input
            type="range"
            className="eth-kbmidi-pop__slider"
            min={MIN_VELOCITY}
            max={MAX_VELOCITY}
            step={1}
            value={velocity}
            aria-label={t("keyboard_midi.velocity")}
            onChange={(e) => setVelocity(Number(e.currentTarget.value))}
          />
          <span className="eth-kbmidi-pop__value" data-testid="keyboard-midi-velocity">
            {velocity}
          </span>
        </label>
        <div className="eth-kbmidi-pop__presets" role="group" aria-label={t("keyboard_midi.velocity_presets", { keys: "C/V" })}>
          {velocityStops().map((v) => (
            <button key={v} type="button" className="eth-kbmidi-pop__preset" aria-pressed={v === velocity} onClick={() => setVelocity(v)}>
              {v}
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}
