import clsx from "clsx";
import { useState, type DragEvent } from "react";
import type { Device, DeviceId } from "@/generated";
import { PluginDeviceControls } from "@/features/plugins";
import { Button } from "@/kit";
import { cmd } from "@/transport";
import { DEVICE_DRAG_TYPE, groupParams } from "./chainUtils";
import { useDescriptor } from "./descriptors";
import { useGestureSender, useSend } from "./gesture";
import { ParamControl } from "./ParamControl";

export interface DeviceViewProps {
  device: Device;
  /** Id of the previous device in the chain (for "move left"), if any. */
  prev: DeviceId | null;
  /** `before` target for "move right" (the device after next, or `null` = end); `undefined` = already last. */
  moveRightBefore: DeviceId | null | undefined;
  /** Drop handler for a dragged device, inserted before this one. */
  onDropBefore(dragged: DeviceId): void;
}

export function DeviceView({ device, prev, moveRightBefore, onDropBefore }: DeviceViewProps) {
  const send = useSend();
  const sender = useGestureSender();
  const { descriptor, error } = useDescriptor(device);
  const [dropTarget, setDropTarget] = useState(false);
  const move = (before: DeviceId | null) =>
    void send(cmd("Device", { type: "Move", id: device.id, track: device.track, before }));

  const onDragOver = (e: DragEvent) => {
    if (!e.dataTransfer.types.includes(DEVICE_DRAG_TYPE)) return;
    e.preventDefault();
    setDropTarget(true);
  };
  const onDrop = (e: DragEvent) => {
    setDropTarget(false);
    const id = e.dataTransfer.getData(DEVICE_DRAG_TYPE);
    if (!id || id === device.id) return;
    e.preventDefault();
    onDropBefore(id);
  };

  return (
    <section
      className={clsx("eth-device", !device.enabled && "eth-device--bypassed", dropTarget && "eth-device--drop")}
      data-device={device.id}
      aria-label={device.name}
      onDragOver={onDragOver}
      onDragLeave={() => setDropTarget(false)}
      onDrop={onDrop}
    >
      <header
        className="eth-device__header"
        draggable
        onDragStart={(e) => {
          e.dataTransfer.setData(DEVICE_DRAG_TYPE, device.id);
          e.dataTransfer.effectAllowed = "move";
        }}
      >
        <Button
          size="sm"
          variant="ghost"
          className="eth-device__power"
          active={device.enabled}
          aria-label={device.enabled ? `Bypass ${device.name}` : `Enable ${device.name}`}
          title={device.enabled ? "Device on (click to bypass)" : "Device bypassed (click to enable)"}
          onClick={() => void send(cmd("Device", { type: "SetEnabled", id: device.id, enabled: !device.enabled }))}
        >
          ⏻
        </Button>
        <span className="eth-device__name">{device.name}</span>
        <PluginDeviceControls device={device} />
        <Button
          size="sm"
          variant="ghost"
          aria-label={`Move ${device.name} left`}
          disabled={prev === null}
          onClick={() => prev !== null && move(prev)}
        >
          ◀
        </Button>
        <Button
          size="sm"
          variant="ghost"
          aria-label={`Move ${device.name} right`}
          disabled={moveRightBefore === undefined}
          onClick={() => moveRightBefore !== undefined && move(moveRightBefore)}
        >
          ▶
        </Button>
        <Button
          size="sm"
          variant="ghost"
          aria-label={`Remove ${device.name}`}
          onClick={() => void send(cmd("Device", { type: "Remove", id: device.id }))}
        >
          ✕
        </Button>
      </header>
      <div className="eth-device__body">
        {descriptor ? (
          groupParams(descriptor.params).map(({ group, params }) => (
            <div className="eth-device__group" key={group ?? ""}>
              {group && <div className="eth-device__group-name">{group}</div>}
              <div className="eth-device__params">
                {params.map((p) => (
                  <ParamControl key={p.id} device={device} info={p} sender={sender} />
                ))}
              </div>
            </div>
          ))
        ) : (
          <div className="eth-device__status">{error ? "Descriptor unavailable" : "Loading…"}</div>
        )}
      </div>
    </section>
  );
}
