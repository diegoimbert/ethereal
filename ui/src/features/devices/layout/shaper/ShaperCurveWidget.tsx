/**
 * One lane of the shaper (`Widget::ShaperCurve`): the lane's drawn cycle
 * (`BuiltinDevice::Shaper::curves`) in the shared curve editor, always tempo-synced (its
 * `rate` choice sets the grid and the song-locked playhead), with what the device plays at
 * the lane's depth drawn under it. Edits are `Device::SetShaperCurve` undo steps.
 */

import type { Device, LfoCurve, ShaperLane, Widget, WidgetSize } from "@/generated";
import { cmd } from "@/transport";
import { useLayoutContext, useParam } from "../context";
import { CurveEditor, DEFAULT_SHAPER_CURVES } from "../lfoCurve";
import { SYNC_RATE_BEATS } from "../lfoCurve/lfoCurveMath";
import { TypedFrame } from "../widgets/typed";

type Spec = Extract<Widget, { type: "ShaperCurve" }>;

const KEY = { Volume: "volume", Cutoff: "cutoff", Pan: "pan" } as const satisfies Record<ShaperLane, string>;

/** The curve of `lane` of a shaper (the defaults when absent). */
function shaperCurveOf(device: Device, lane: ShaperLane): LfoCurve {
  const k = device.kind;
  const curves = k.type === "Builtin" && k.device.type === "Shaper" && k.device.curves ? k.device.curves : DEFAULT_SHAPER_CURVES;
  return curves[KEY[lane]];
}

export function ShaperCurveWidget({ widget: w, size, label }: { widget: Spec; size: WidgetSize; label: string | null }) {
  const { sender, device } = useLayoutContext();
  const on = useParam(w.on);
  const rate = useParam(w.rate);
  const depth = useParam(w.depth);
  const d = depth ? depth.plain / 100 : 1;
  // Pan plays around the centre; volume and cutoff down from the top.
  const played = d >= 1 ? null : w.lane === "Pan" ? (y: number) => 0.5 + d * (y - 0.5) : (y: number) => 1 - d * (1 - y);
  return (
    <TypedFrame type={`shaper-${KEY[w.lane]}`} size={size} label={label}>
      <CurveEditor
        curve={shaperCurveOf(device, w.lane)}
        send={(curve) => void sender.send(cmd("Device", { type: "SetShaperCurve", device: device.id, lane: w.lane, curve }))}
        cycleBeats={SYNC_RATE_BEATS[Math.round(rate?.plain ?? 8)] ?? 1}
        hz={1}
        offset={0}
        played={played}
        off={on ? on.plain < 0.5 : false}
        testId={`widget-shaper-${KEY[w.lane]}`}
      />
    </TypedFrame>
  );
}
