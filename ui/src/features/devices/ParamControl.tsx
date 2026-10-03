import type { DeviceId, ParamInfo } from "@/generated";
import { Button, Knob, Select } from "@/kit";
import { cmd } from "@/transport";
import { midiTarget } from "@/features/midi-learn/targets";
import type { GestureSender } from "./gesture";
import { formatParam, labelIndex, labelValue, paramToNormalized, paramToPlain } from "./paramScale";

export interface ParamControlProps {
  device: DeviceId;
  /** The param's document (plain) value. */
  plain: number;
  info: ParamInfo;
  sender: GestureSender;
  /** Main controls are large (value inside the ring); folded ones small with a value line. */
  size?: "sm" | "md" | "lg";
}

/** Generic control for one param, chosen from its `ParamInfo` (knob, toggle or choice). */
export function ParamControl({ device, plain, info, sender, size = "md" }: ParamControlProps) {
  const setPlain = (value: number) =>
    void sender.send(cmd("Device", { type: "SetParam", device, param: info.id, value }));
  const labels = info.labels;
  const target = midiTarget({ type: "Param", target: { type: "DeviceParam", device, param: info.id } });

  if (labels?.length === 2 || (info.unit === "Toggle" && !labels)) {
    const on = labels ? labelIndex(info, plain) === 1 : plain >= (info.min + info.max) / 2;
    const onOff = labels ?? ["Off", "On"];
    return (
      <div className="eth-param eth-param--toggle" data-param={info.id} {...target}>
        <Button
          size="sm"
          active={on}
          aria-label={info.name}
          onClick={() => setPlain(on ? info.min : info.max)}
        >
          {onOff[on ? 1 : 0]}
        </Button>
        <span className="eth-param__name">{info.name}</span>
      </div>
    );
  }

  if (labels && labels.length > 2) {
    return (
      <label className="eth-param eth-param--choice" data-param={info.id} {...target}>
        <Select
          size="sm"
          className="eth-param__select"
          aria-label={info.name}
          value={String(labelIndex(info, plain))}
          onChange={(v) => setPlain(labelValue(info, Number(v)))}
          options={labels.map((l, i) => ({ value: String(i), label: l }))}
        />
        <span className="eth-param__name">{info.name}</span>
      </label>
    );
  }

  const text = formatParam(info, plain);
  return (
    <div className={`eth-param eth-param--knob eth-param--${size}`} data-param={info.id} {...target}>
      <Knob
        size={size}
        value={paramToNormalized(info, plain)}
        defaultValue={paramToNormalized(info, info.default)}
        bipolar={info.min < 0 && info.max > 0}
        label={info.name}
        hideLabel={size === "sm"}
        valueText={text}
        onChange={(n) => setPlain(paramToPlain(info, n))}
        onChangeStart={sender.begin}
        onChangeEnd={sender.end}
      />
      {size !== "lg" && <span className="eth-param__value">{text}</span>}
    </div>
  );
}
