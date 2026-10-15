import type { Track } from "@/generated";
import { useGestureSender } from "@/features/devices/gesture";
import { formatPan, paramToPlain } from "@/domain/paramScale";
import { useLiveAutomation } from "@/features/automation/live";
import { PAN_INFO } from "@/features/automation/params";
import { midiTarget } from "@/features/midi-learn/targets";
import { Knob } from "@/kit";
import { cmd } from "@/transport";
import { useT } from "@/i18n";

/**
 * Pan knob in a track header, right of the volume fader. Drag up / down as one undo step;
 * double-click centers it. Follows pan automation while playing.
 */
export function HeaderPan({ track }: { track: Track }) {
  const t = useT();
  const sender = useGestureSender();
  const base = track.mixer.pan;
  const live = useLiveAutomation({ type: "TrackPan", track: track.id });
  const pan = live === null ? base : paramToPlain(PAN_INFO, live);
  const set = (v: number) => {
    const next = Math.round((Math.min(1, Math.max(0, v)) * 2 - 1) * 100) / 100;
    if (next !== base) void sender.send(cmd("Mixer", { type: "SetPan", track: track.id, pan: next }));
  };
  return (
    // Not a header press (drag / selection) nor a click on it.
    <span className="eth-arr-pan" onPointerDown={(e) => e.stopPropagation()} onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
      <Knob
        size="sm"
        bipolar
        hideLabel
        label={t("arrangement.header.pan", { name: track.name })}
        value={(pan + 1) / 2}
        valueText={formatPan(pan)}
        onChange={set}
        onChangeStart={() => sender.begin()}
        onChangeEnd={() => sender.end()}
        {...midiTarget({ type: "Param", target: { type: "TrackPan", track: track.id } })}
      />
    </span>
  );
}
