import clsx from "clsx";
import { useState, type DragEvent } from "react";
import type { Device, DeviceDescriptor, DeviceId } from "@/generated";
import { PluginDeviceControls } from "@/features/plugins";
import { PresetMenu } from "@/features/presets";
import { SidechainSelector } from "@/features/sidechain";
import { ChevronDown, ChevronLeft, ChevronRight, ChevronUp, Power, X } from "lucide-react";
import { IconButton, openContextMenu } from "@/kit";
import { cmd } from "@/transport";
import { DEVICE_DRAG_TYPE } from "./chainUtils";
import { useDescriptor } from "./descriptors";
import { useGestureSender, useSend } from "./gesture";
import { SampleSlot } from "./SampleSlot";
import { DeviceLayoutView } from "./layout";
import { setCollapsed, useCollapsed } from "./collapsed";

export interface DeviceViewProps {
  device: Device;
  /** Id of the previous device in the chain (for "move left"), if any. */
  prev: DeviceId | null;
  /** `before` target for "move right" (the device after next, or `null` = end); `undefined` = already last. */
  moveRightBefore: DeviceId | null | undefined;
  /** Drop handler for a dragged device, inserted before this one. */
  onDropBefore(dragged: DeviceId): void;
  /**
   * "row" (default): cards side by side, move left/right. "stack": full-width cards one
   * above the other (the inspector), move up/down, and a collapse chevron that animates
   * the card body closed (remembered per device).
   */
  layout?: "row" | "stack";
}

export function DeviceView({ device, prev, moveRightBefore, onDropBefore, layout = "row" }: DeviceViewProps) {
  const stack = layout === "stack";
  const collapsed = useCollapsed(device.id) && stack;
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
      className={clsx(
        "eth-device",
        stack && "eth-device--stack",
        collapsed && "eth-device--collapsed",
        !device.enabled && "eth-device--bypassed",
        dropTarget && "eth-device--drop",
      )}
      data-device={device.id}
      aria-label={device.name}
      onDragOver={onDragOver}
      onDragLeave={() => setDropTarget(false)}
      onDrop={onDrop}
    >
      <header
        className="eth-device__header"
        onContextMenu={(e) =>
          openContextMenu(e, [
            {
              label: device.enabled ? "Bypass" : "Enable",
              onSelect: () => void send(cmd("Device", { type: "SetEnabled", id: device.id, enabled: !device.enabled })),
            },
            "separator",
            { label: "Delete Device", danger: true, onSelect: () => void send(cmd("Device", { type: "Remove", id: device.id })) },
          ])
        }
        draggable
        onDragStart={(e) => {
          e.dataTransfer.setData(DEVICE_DRAG_TYPE, device.id);
          e.dataTransfer.effectAllowed = "move";
        }}
      >
        {stack && (
          <IconButton
            size="sm"
            tone="ghost"
            className="eth-device__collapse-toggle"
            aria-expanded={!collapsed}
            label={collapsed ? `Expand ${device.name}` : `Collapse ${device.name}`}
            icon={<ChevronDown />}
            onClick={() => setCollapsed(device.id, !collapsed)}
          />
        )}
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-device__power"
          active={device.enabled}
          label={device.enabled ? `Bypass ${device.name}` : `Enable ${device.name}`}
          title={device.enabled ? "Device on (click to bypass)" : "Device bypassed (click to enable)"}
          icon={<Power />}
          onClick={() => void send(cmd("Device", { type: "SetEnabled", id: device.id, enabled: !device.enabled }))}
        />
        <span className="eth-device__name">{device.name}</span>
        <PresetMenu device={device} />
        <PluginDeviceControls device={device} />
        <SidechainSelector device={device} />
        <span className="eth-device__actions">
          <IconButton
            size="sm"
            tone="ghost"
            label={`Move ${device.name} ${stack ? "up" : "left"}`}
            icon={stack ? <ChevronUp /> : <ChevronLeft />}
            disabled={prev === null}
            onClick={() => prev !== null && move(prev)}
          />
          <IconButton
            size="sm"
            tone="ghost"
            label={`Move ${device.name} ${stack ? "down" : "right"}`}
            icon={stack ? <ChevronDown /> : <ChevronRight />}
            disabled={moveRightBefore === undefined}
            onClick={() => moveRightBefore !== undefined && move(moveRightBefore)}
          />
          <IconButton
            size="sm"
            tone="ghost"
            label={`Remove ${device.name}`}
            icon={<X />}
            onClick={() => void send(cmd("Device", { type: "Remove", id: device.id }))}
          />
        </span>
      </header>
      {/* The card body; in the stacked layout it collapses (animated height). */}
      <div className={clsx("eth-device__collapse", collapsed && "eth-device__collapse--closed")} inert={collapsed}>
        <div className="eth-device__collapse-inner">
          {/* The sample drop slot, unless the layout's waveform widget carries it. */}
          {!hasWaveform(descriptor) && <SampleSlot device={device} />}
          {descriptor ? (
            // The one shared renderer: the declared layout, or the generic one.
            <DeviceLayoutView device={device} descriptor={descriptor} sender={sender} />
          ) : (
            <div className="eth-device__body">
              <div className="eth-device__status">{error ? "Descriptor unavailable" : "Loading…"}</div>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

/** Does the device's layout draw the sample waveform (with its own drop slot)? */
function hasWaveform(descriptor: DeviceDescriptor | null): boolean {
  return !!descriptor?.layout?.sections.some((s) => s.items.some((i) => i.widget.type === "SampleWaveform"));
}
