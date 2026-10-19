import "./keyboard-midi.css";
import { useContext, useEffect } from "react";
import { Piano } from "lucide-react";
import { Button } from "@/kit";
import { useShortcutLabel } from "@/features/keymap";
import { useProjectStore } from "@/state";
import { TransportContext } from "@/transport";
import { bindTransport, targetTrack, toggleKeyboardMidi, TOGGLE_ACTION, useKeyboardMidi } from "./keyboard";
import { noteName, octaveBase } from "./notes";
import { useT } from "@/i18n";
import { KeyboardMidiPopover } from "./KeyboardMidiPopover";

/** The keys named in the tooltip (key caps, not translated). */
const KEYS = { keys: "A–;", octave: "Z/X", vel: "C/V" }; // i18n-ignore

/**
 * Transport-bar toggle of the computer MIDI keyboard, with its indicator while on: the
 * lowest note (octave, Z/X) and the velocity (C/V), a button opening a popover to set both
 * (`KeyboardMidiPopover`). Lit while notes are held. Binds the
 * app's transport for the key handler (`keyboard.ts`).
 */
export function KeyboardMidiButton({ className }: { className?: string }) {
  const transport = useContext(TransportContext)?.transport ?? null;
  useEffect(() => bindTransport(transport), [transport]);
  const enabled = useKeyboardMidi((s) => s.enabled);
  const octave = useKeyboardMidi((s) => s.octave);
  const velocity = useKeyboardMidi((s) => s.velocity);
  const playing = useKeyboardMidi((s) => Object.keys(s.held).length > 0);
  const target = useProjectStore((s) => (s.project ? targetTrack(s.project, s.armedTracks)?.name : undefined));
  const shortcut = useShortcutLabel(TOGGLE_ACTION);
  const t = useT();

  const low = noteName(octaveBase(octave));
  const title = enabled
    ? `${shortcut ? t("keyboard_midi.title_on_key", { shortcut, low, ...KEYS }) : t("keyboard_midi.title_on", { low, ...KEYS })}. ${
        target ? t("keyboard_midi.plays_on", { track: target }) : t("keyboard_midi.no_track")
      }`
    : shortcut
      ? t("keyboard_midi.title_off_key", { shortcut })
      : t("keyboard_midi.title_off");

  return (
    <span className="eth-kbmidi" data-feature="keyboard-midi" data-enabled={enabled || undefined}>
      <Button
        tone="ghost"
        aria-label={t("keyboard_midi.label")}
        title={title}
        active={enabled}
        className={className ? `${className} eth-kbmidi__toggle` : "eth-kbmidi__toggle"}
        data-testid="keyboard-midi-toggle"
        onClick={toggleKeyboardMidi}
      >
        <Piano aria-hidden />
      </Button>
      {enabled && <KeyboardMidiPopover low={low} velocity={velocity} playing={playing} />}
    </span>
  );
}
