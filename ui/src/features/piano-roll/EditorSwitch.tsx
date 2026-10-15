import type { TrackId } from "@/generated";
import { useT } from "@/i18n";
import { Button } from "@/kit";
import { useBeatView } from "./beatView";

/**
 * A Drum Rack track's editor choice (beat editor | piano roll): the drawer's own pill tabs
 * (`eth-shell__tab`), at the far right of both toolbars, so the control and its place are the
 * same in either editor.
 */
export function EditorSwitch({ track }: { track: TrackId }) {
  const t = useT();
  const pianoRoll = useBeatView((s) => s.pianoRoll.has(track));
  const set = (on: boolean) => useBeatView.getState().setPianoRoll(track, on);
  return (
    <span className="eth-shell__tabs eth-pr__switch" role="tablist" aria-label={t("piano_roll.editor_switch")}>
      <Button size="sm" tone="ghost" role="tab" className="eth-shell__tab" aria-selected={!pianoRoll} onClick={() => set(false)}>
        {t("piano_roll.tab.beat_editor")}
      </Button>
      <Button size="sm" tone="ghost" role="tab" className="eth-shell__tab" aria-selected={pianoRoll} onClick={() => set(true)}>
        {t("piano_roll.tab.piano_roll")}
      </Button>
    </span>
  );
}
