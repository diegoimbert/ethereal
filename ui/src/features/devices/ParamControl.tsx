import type { Device, ParamInfo } from "@/generated";
import { Button, Knob } from "@/kit";
import { cmd } from "@/transport";
import type { GestureSender } from "./gesture";
import { formatParam, labelIndex, labelValue, paramToNormalized, paramToPlain } from "./paramScale";

export interface ParamControlProps {
  device: Device;
  info: ParamInfo;
  sender: GestureSender;
}

/** Generic control for one param, chosen from its `ParamInfo` (knob, toggle or choice). */
export function ParamControl({ device, info, sender }: ParamControlProps) {
  const plain = device.params[info.id] ?? info.default;
  const setPlain = (value: number) =>
    void sender.send(cmd("Device", { type: "SetParam", device: device.id, param: info.id, value }));
  const labels = info.labels;

  if (labels?.length === 2 || (info.unit === "Toggle" && !labels)) {
    const on = labels ? labelIndex(info, plain) === 1 : plain >= (info.min + info.max) / 2;
    const onOff = labels ?? ["Off", "On"];
    return (
      <div className="eth-param eth-param--toggle" data-param={info.id}>
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
      <label className="eth-param eth-param--choice" data-param={info.id}>
        <select
          className="eth-param__select"
          aria-label={info.name}
          value={labelIndex(info, plain)}
          onChange={(e) => setPlain(labelValue(info, Number(e.target.value)))}
        >
          {labels.map((l, i) => (
            <option key={i} value={i}>
              {l}
            </option>
          ))}
        </select>
        <span className="eth-param__name">{info.name}</span>
      </label>
    );
  }

  const text = formatParam(info, plain);
  return (
    <div className="eth-param eth-param--knob" data-param={info.id}>
      <Knob
        value={paramToNormalized(info, plain)}
        defaultValue={paramToNormalized(info, info.default)}
        bipolar={info.min < 0 && info.max > 0}
        label={info.name}
        valueText={text}
        onChange={(n) => setPlain(paramToPlain(info, n))}
        onChangeStart={sender.begin}
        onChangeEnd={sender.end}
      />
      <span className="eth-param__value">{text}</span>
    </div>
  );
}
