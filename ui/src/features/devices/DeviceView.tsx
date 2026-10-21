import clsx from "clsx";
import { useContributions } from "@/ext";
import { useId, useState, type DragEvent } from "react";
import type { Device, DeviceId } from "@/generated";
import { PluginDeviceControls, pluginMenuEntries } from "@/features/plugins";
import { MissingDeviceControls, ReplacePopover, replaceDeviceEntries, resolvePluginEntries } from "@/features/import-resolver";
import { PresetMenu } from "@/features/presets";
import { SidechainSelector } from "@/features/sidechain";
import { AddModulatorButton, addModulatorEntries, ModulatorsPanel, useModulatorKinds } from "@/features/modulation";
import { groupEntries, isChainRack, RackPanel } from "@/features/racks";
import { freezeDeviceEntries, useDeviceFrozen } from "@/features/freeze";
import { ChevronLeft, ChevronRight, Power, SlidersHorizontal, Snowflake, X } from "lucide-react";
import { IconButton, openContextMenu, type ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { useDeviceOn } from "@/features/automation/live";
import { midiTarget } from "@/features/midi-learn/targets";
import { switchMenu, switchTarget } from "./layout/automation";
import { cmd, useTransport } from "@/transport";
import { useT } from "@/i18n";
import { DEVICE_DRAG_TYPE, currentChainDrag, deviceCategory, droppedChainDrag, isChainDrag, setDraggingDevice, type ChainDrag } from "./chainUtils";
import { copyFrom, pasteDevice, selectModeOf, useDeviceClipboard, useDeviceClipboardKeys } from "./deviceClipboard";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { shortcutLabel } from "@/features/keymap";
import { useBuiltinTypes, useDescriptor } from "./descriptors";
import { useGestureSender, useSend } from "./gesture";
import { setParamsOpen, useParamsOpen } from "./collapsed";
import { DeviceParams } from "./params/DeviceParams";
import { pinKey } from "./params/pins";
import { DeviceWindowButton } from "./window/DeviceWindowButton";
import { hasWindow, useDeviceWindow } from "./window/useDeviceWindow";

export interface DeviceViewProps {
  device: Device;
  /** Id of the previous device in the chain (for "move left"), if any. */
  prev: DeviceId | null;
  /** `before` target for "move right" (the device after next, or `null` = end); `undefined` = already last. */
  moveRightBefore: DeviceId | null | undefined;
  /** Drop handler for a dragged device (or a library device), inserted before this one. */
  onDropBefore(dragged: ChainDrag): void;
  /**
   * Whether what is dragged (`null`: unknown, e.g. from another window) may land before this
   * one; refused drops show no insertion line. Default: any.
   */
  acceptsBefore?(dragged: ChainDrag | null): boolean;
  /**
   * "row" (default): cards side by side, move left/right. "stack": full-width cards one
   * above the other (the inspector), reordered by dragging the header.
   */
  layout?: "row" | "stack";
  /** v0.2: move handler for rack-chain devices (default `Device::Move`). */
  onMove?(before: DeviceId | null): void;
  /** Remove handler (default `Device::Remove`), e.g. a drum pad's last device removes the pad. */
  onRemove?(): void;
}

export function DeviceView({ device, prev, moveRightBefore, onDropBefore, acceptsBefore, layout = "row", onMove, onRemove }: DeviceViewProps) {
  const stack = layout === "stack";
  const t = useT();
  // Two independent states. The window (the plugin's editor, or the factory
  // device's own UI) opens from the header's window button; the inline parameter list (the
  // generic UI, capped height) from its "Parameters" button.
  const paramsOpen = useParamsOpen(device.id);
  const deviceWindow = useDeviceWindow(device);
  const paramsId = useId();
  const send = useSend();
  const remove = () => (onRemove ? onRemove() : void send(cmd("Device", { type: "Remove", id: device.id })));
  const { on, automated } = useDeviceOn(device);
  // Freeze (#25): a frozen device (its track plays a render through it) is locked; with a
  // partial freeze, the devices after the freeze point stay live.
  const frozen = useDeviceFrozen(device);
  const headerBadges = useContributions("devices.headerBadge");
  const setEnabled = (enabled: boolean) => void send(cmd("Device", { type: "SetEnabled", id: device.id, enabled }));
  const transport = useTransport();
  // Copy / paste (deviceClipboard.ts): the selected card, what's copied, device kinds.
  const selected = useDeviceClipboard((s) => s.selected.includes(device.id));
  const copied = useDeviceClipboard((s) => s.copied.length > 0);
  const isCut = useDeviceClipboard((s) => s.cut && s.copied.includes(device.id));
  const builtins = useBuiltinTypes();
  const plugins = usePluginStore((s) => s.plugins);
  const categoryOf = (d: Device) => deviceCategory(d, builtins, plugins);
  useDeviceClipboardKeys(transport, categoryOf);
  const sender = useGestureSender();
  const { descriptor, error } = useDescriptor(device);
  const held = useProjectStore((s) => s.safeMode.includes(device.id));
  const [dropTarget, setDropTarget] = useState(false);
  const [renaming, setRenaming] = useState(false);
  /** A name for this instance; empty brings back the device's own name (its type's or plugin's). */
  const rename = (name: string) => {
    setRenaming(false);
    const next = name.trim() || descriptor?.name || device.name;
    if (next !== device.name) void send(cmd("Device", { type: "Rename", id: device.id, name: next }));
  };
  const modulatorKinds = useModulatorKinds();
  const move = (before: DeviceId | null) => (onMove ? onMove(before) : void send(cmd("Device", { type: "Move", id: device.id, track: device.track, before })));

  const onDragOver = (e: DragEvent) => {
    if (!isChainDrag(e.dataTransfer.types)) return;
    const drag = currentChainDrag();
    if ((drag?.type === "device" && drag.id === device.id) || (acceptsBefore && !acceptsBefore(drag))) return;
    e.preventDefault();
    e.stopPropagation();
    setDropTarget(true);
  };
  const onDrop = (e: DragEvent) => {
    setDropTarget(false);
    const drag = droppedChainDrag(e.dataTransfer);
    if (!drag || (drag.type === "device" && drag.id === device.id)) return;
    e.preventDefault();
    e.stopPropagation();
    onDropBefore(drag);
  };

  return (
    <>
      {/* Where a dragged device lands: a line in the gap before this card. */}
      {dropTarget && <div className="eth-device__insert" aria-hidden data-testid="device-insert" />}
      <section
        className={clsx(
          "eth-device",
          stack && "eth-device--stack",
          !paramsOpen && "eth-device--collapsed",
          !on && "eth-device--bypassed",
          selected && "eth-device--selected",
          isCut && "eth-device--cut",
          frozen && "eth-device--frozen",
        )}
        data-frozen={frozen || undefined}
        data-device={device.id}
        aria-label={device.name}
        // A click selects the card (the innermost one: a device in a rack chain, not its rack)
        // and focuses it, so ⌘C / ⌘V copy / paste it (deviceClipboard.ts); ⌘ / Ctrl-click adds
        // or removes it, Shift-click selects the cards between.
        tabIndex={-1}
        onPointerDown={(e) => {
          // A rack chain's own space inside this card is that chain's (DeviceChain.tsx).
          if (e.button !== 0 || (e.target as HTMLElement).closest(".eth-device, [data-paste-chain]") !== e.currentTarget) return;
          useDeviceClipboard.getState().select(device.id, selectModeOf(e));
          if (!(e.target as HTMLElement).closest("input, textarea, select, [contenteditable='true']")) e.currentTarget.focus({ preventScroll: true });
        }}
        onDragOver={onDragOver}
        onDragLeave={() => setDropTarget(false)}
        onDrop={onDrop}
      >
        <header
          className="eth-device__header"
          onContextMenu={(e) =>
            openContextMenu(
              e,
              // Groups, separated: the device itself; racks (v0.2 racks and modulation); modulators;
              // plugin (advanced); delete.
              menuGroups([
                // A plugin that isn't installed; any device but a rack can be replaced.
                [...resolvePluginEntries(device, transport), ...replaceDeviceEntries(device, transport)],
                [
                  { label: t("devices.device.copy"), shortcut: shortcutLabel("edit.copy"), onSelect: () => copyFrom(device.id) },
                  { label: t("devices.device.cut"), shortcut: shortcutLabel("edit.cut"), onSelect: () => copyFrom(device.id, true) },
                  ...(copied
                    ? [
                        {
                          label: t("devices.device.paste_after"),
                          shortcut: shortcutLabel("edit.paste"),
                          onSelect: () => void pasteDevice(transport, device.id, categoryOf),
                        },
                      ]
                    : []),
                ],
                [
                  { label: t("devices.device.rename_menu"), onSelect: () => setRenaming(true) },
                  {
                    label: device.enabled ? t("devices.device.bypass") : t("devices.device.enable"),
                    onSelect: () => setEnabled(!device.enabled),
                  },
                ],
                groupEntries(device, descriptor, send, {
                  project: useProjectStore.getState().project!,
                  ids: useDeviceClipboard.getState().selected,
                  categoryOf,
                  shortcut: shortcutLabel("device.group"),
                  onGrouped: (rack) => useDeviceClipboard.setState({ selected: [rack], anchor: rack }),
                }),
                freezeDeviceEntries(transport, device, categoryOf),
                device.pad === null && device.chain == null ? addModulatorEntries(modulatorKinds, device, send) : [],
                pluginMenuEntries(device, transport),
                [{ label: t("devices.device.delete"), danger: true, onSelect: remove }],
              ]),
            )
          }
          draggable={!renaming}
          onDragStart={(e) => {
            e.dataTransfer.setData(DEVICE_DRAG_TYPE, device.id);
            // Moved in its chain or onto a track in the arranger (⌘ / Ctrl there copies).
            e.dataTransfer.effectAllowed = "copyMove";
            setDraggingDevice(device.id);
          }}
          onDragEnd={() => setDraggingDevice(null)}
        >
          <IconButton
            size="sm"
            tone="ghost"
            className="eth-device__power"
            active={on}
            label={device.enabled ? t("devices.device.bypass_name", { name: device.name }) : t("devices.device.enable_name", { name: device.name })}
            title={
              automated
                ? t("devices.device.power_automated", { state: on ? "on" : "bypassed", action: device.enabled ? "bypass" : "enable" })
                : device.enabled
                  ? t("devices.device.power_on")
                  : t("devices.device.power_bypassed")
            }
            data-automated={automated || undefined}
            icon={<Power />}
            onClick={() => setEnabled(!device.enabled)}
            // Like a knob: right-click for its automation lane and reset; MIDI-learnable.
            onContextMenu={(e) => switchMenu(e, device, setEnabled)}
            {...midiTarget({ type: "Param", target: switchTarget(device) })}
          />
          {/* After the power button. A chain rack has no window (its macros and chains are on
              this card). */}
          {hasWindow(device) && <DeviceWindowButton device={device} control={deviceWindow} />}
          {renaming ? (
            <input
              className="eth-device__rename"
              aria-label={t("devices.device.rename", { name: device.name })}
              placeholder={descriptor?.name ?? device.name}
              defaultValue={device.name}
              autoFocus
              onFocus={(e) => e.currentTarget.select()}
              onBlur={(e) => rename(e.currentTarget.value)}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") e.currentTarget.blur();
                if (e.key === "Escape") setRenaming(false);
              }}
            />
          ) : (
            <span className="eth-device__name eth-user-text" title={t("devices.device.rename_hint")} onDoubleClick={() => setRenaming(true)}>
              {device.name}
            </span>
          )}
          {/* Presets and "+" (modulator), right after the name: they take no room until the
              header is hovered or focused, so the name uses the whole width. */}
          <span className="eth-device__on-hover" data-testid="device-on-hover">
            <span className="eth-device__on-hover-inner">
              <PresetMenu device={device} />
              <AddModulatorButton device={device} />
            </span>
          </span>
          {/* Extensions' badges (`@/ext` "devices.headerBadge"). */}
          {headerBadges.map((Badge, i) => (
            <Badge key={i} device={device} />
          ))}
          {frozen && (
            <span className="eth-device__frozen" role="img" aria-label={t("devices.device.frozen")} title={t("devices.device.frozen_title")}>
              <Snowflake aria-hidden />
            </span>
          )}
          <PluginDeviceControls device={device} />
          <MissingDeviceControls device={device} />
          <SidechainSelector device={device} />
          <span className="eth-device__actions">
            {/* Stacked (inspector): reorder by dragging the header; side by side: left / right. */}
            {!stack && (
              <>
                <IconButton
                  size="sm"
                  tone="ghost"
                  label={t("devices.device.move_left", { name: device.name })}
                  icon={<ChevronLeft />}
                  disabled={prev === null}
                  onClick={() => prev !== null && move(prev)}
                />
                <IconButton
                  size="sm"
                  tone="ghost"
                  label={t("devices.device.move_right", { name: device.name })}
                  icon={<ChevronRight />}
                  disabled={moveRightBefore === undefined}
                  onClick={() => moveRightBefore !== undefined && move(moveRightBefore)}
                />
              </>
            )}
            <IconButton
              size="sm"
              tone="ghost"
              className="eth-device__params-toggle"
              data-viewer-allow
              active={paramsOpen}
              aria-expanded={paramsOpen}
              aria-controls={paramsOpen ? paramsId : undefined}
              label={t("devices.device.show_params_name", { name: device.name })}
              title={paramsOpen ? t("devices.device.hide_params") : t("devices.device.show_params")}
              icon={<SlidersHorizontal />}
              onClick={() => setParamsOpen(device.id, !paramsOpen)}
            />
            <IconButton size="sm" tone="ghost" label={t("devices.device.remove", { name: device.name })} icon={<X />} onClick={remove} />
          </span>
        </header>
        {/* "Replace device…" (the header's menu): its popover hangs here, under the header. */}
        <ReplacePopover device={device} />
        {/* The card body: the parameter list (when shown), a rack's chains, the modulators.
            The device's own UI is in its window, never here. */}
        <div className="eth-device__inline" inert={frozen || undefined}>
          {paramsOpen &&
            (descriptor ? (
              <DeviceParams id={paramsId} device={device.id} deviceName={device.name} descriptor={descriptor} sender={sender} pins={pinKey(device)} />
            ) : (
              <div className="eth-device__body" id={paramsId}>
                <div className="eth-device__status">
                  {held ? t("devices.device.not_loaded_safe_mode") : error ? t("devices.device.descriptor_unavailable") : t("devices.device.loading")}
                </div>
              </div>
            ))}
          {/* v0.2: a rack's chains, then the device's modulators. */}
          {isChainRack(device) && <RackPanel rack={device} />}
          <ModulatorsPanel device={device} />
        </div>
      </section>
    </>
  );
}

/** Menu groups joined by separators (empty groups dropped). */
function menuGroups(groups: ReadonlyArray<ReadonlyArray<ContextMenuEntry>>): ContextMenuEntry[] {
  return groups.filter((g) => g.length > 0).flatMap((g, i) => (i === 0 ? [...g] : ["separator" as const, ...g]));
}
