import clsx from "clsx";
import { useState, type DragEvent, type ReactNode } from "react";
import type { Device, DeviceId } from "@/generated";
import { PluginDeviceControls } from "@/features/plugins";
import { SidechainSelector } from "@/features/sidechain";
import { ChevronDown, ChevronLeft, ChevronRight, Power, X } from "lucide-react";
import { IconButton, openContextMenu } from "@/kit";
import { cmd } from "@/transport";
import { DEVICE_DRAG_TYPE, groupParams, splitMainParams, type ParamGroup } from "./chainUtils";
import { useDescriptor } from "./descriptors";
import { useGestureSender, useSend } from "./gesture";
import { ParamControl } from "./ParamControl";
import { SampleSlot } from "./SampleSlot";

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
  const [expanded, setExpanded] = useState(false);
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
        <PluginDeviceControls device={device} />
        <SidechainSelector device={device} />
        <span className="eth-device__actions">
          <IconButton
            size="sm"
            tone="ghost"
            label={`Move ${device.name} left`}
            icon={<ChevronLeft />}
            disabled={prev === null}
            onClick={() => prev !== null && move(prev)}
          />
          <IconButton
            size="sm"
            tone="ghost"
            label={`Move ${device.name} right`}
            icon={<ChevronRight />}
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
      <SampleSlot device={device} />
      {descriptor ? (
        <DeviceParams
          groups={groupParams(descriptor.params)}
          name={device.name}
          expanded={expanded}
          onToggle={() => setExpanded((x) => !x)}
          render={(p, size) => <ParamControl key={p.id} device={device} info={p} sender={sender} size={size} />}
        />
      ) : (
        <div className="eth-device__body">
          <div className="eth-device__status">{error ? "Descriptor unavailable" : "Loading…"}</div>
        </div>
      )}
    </section>
  );
}

/**
 * A device's params: the main section (leading groups, large knobs) always shows; the rest
 * folds under a "More" disclosure (see `splitMainParams`).
 */
function DeviceParams({
  groups,
  name,
  expanded,
  onToggle,
  render,
}: {
  groups: ReadonlyArray<ParamGroup>;
  name: string;
  expanded: boolean;
  onToggle(): void;
  render(p: ParamGroup["params"][number], size: "md" | "lg"): ReactNode;
}) {
  const { main, more } = splitMainParams(groups);
  const hidden = more.reduce((n, g) => n + g.params.length, 0);
  const section = (list: ReadonlyArray<ParamGroup>, size: "md" | "lg") =>
    list.map(({ group, params }) => (
      <div className="eth-device__group" key={group ?? ""}>
        {group && <div className="eth-device__group-name">{group}</div>}
        <div className={`eth-device__params eth-device__params--${size}`}>{params.map((p) => render(p, size))}</div>
      </div>
    ));
  return (
    <>
      <div className="eth-device__body">{section(main, "lg")}</div>
      {more.length > 0 && (
        <>
          <button
            type="button"
            className="eth-device__more"
            aria-expanded={expanded}
            aria-label={`${expanded ? "Fewer" : "More"} ${name} controls`}
            onClick={onToggle}
          >
            <ChevronDown className={expanded ? "eth-device__more-icon eth-device__more-icon--open" : "eth-device__more-icon"} />
            {expanded ? "Fewer controls" : `More controls (${hidden})`}
          </button>
          {expanded && <div className="eth-device__body eth-device__body--more">{section(more, "md")}</div>}
        </>
      )}
    </>
  );
}
