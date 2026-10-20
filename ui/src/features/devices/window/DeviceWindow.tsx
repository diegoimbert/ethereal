import clsx from "clsx";
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import type { Device, DeviceDescriptor, DeviceId, Project } from "@/generated";
import { FloatingWindow, ReadOnlySurface, useFloatingWindows } from "@/kit";
import { useProjectStore } from "@/state";
import { useDeviceOn } from "@/features/automation/live";
import { duration, size } from "@/theme/tokens";
import { useDescriptor } from "../descriptors";
import { useGestureSender } from "../gesture";
import { DrumRackEditor, isDrumRack } from "@/features/drum-rack";
import { DeviceLayoutView } from "../layout";
import { SamplePickerWindow, SampleSlot, samplerSample } from "../SampleSlot";
import { deviceWindowWidth } from "./sizes";
import { deviceWindowId, useDeviceWindows } from "./store";
import { hasWindow } from "./useDeviceWindow";
import "./window.css";
import { useT } from "@/i18n";

const INSET = parseFloat(size.deviceWindowInset);
/** Offset between windows opened without a remembered position (cascade). */
const CASCADE = 2 * parseFloat(size.controlMd);

/**
 * The open factory device windows, above the workspace, in kit `FloatingWindow`s
 * (moved, resized, kept on screen, geometry remembered per device). Mounted once by
 * the app shell. A window whose device is removed (deleted, its track deleted) closes; windows
 * of devices of another project stay remembered but aren't shown.
 */
export function DeviceWindowsRoot() {
  const open = useDeviceWindows((s) => s.open);
  const devices = useProjectStore((s) => s.project?.devices);
  useCloseRemovedDevices();
  // Stacking order (the last one on top), drawn with z-index: the DOM order stays put, so
  // raising a window under the pointer never moves it mid-click.
  // Not a chain rack's (no window; one remembered from before isn't restored).
  const shown = open.filter((id) => devices?.[id] && devices[id].kind.type === "Builtin" && hasWindow(devices[id]));
  const key = shown.join(",");
  useEffect(() => {
    // The kit stacks windows in the order they rose: follow the remembered order.
    const kit = useFloatingWindows.getState();
    for (const id of shown) kit.raise(deviceWindowId(id));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  if (shown.length === 0) return null;
  const domOrder = [...shown].sort();
  return createPortal(
    <div className="eth-device-windows" data-testid="device-windows">
      {domOrder.map((id) => (
        <DeviceWindow key={id} device={devices![id]!} stack={shown.indexOf(id)} />
      ))}
    </div>,
    document.body,
  );
}

/** Close the windows of devices removed from the project (not on a project switch). */
function useCloseRemovedDevices(): void {
  useEffect(
    () =>
      useProjectStore.subscribe((s, prev) => {
        const before = prev.project;
        const after = s.project;
        if (!before || !after || before.id !== after.id || before.devices === after.devices) return;
        const gone = removedDevices(before, after);
        if (gone.length > 0) useDeviceWindows.getState().closeMany(gone);
      }),
    [],
  );
}

function removedDevices(before: Project, after: Project): DeviceId[] {
  const open = useDeviceWindows.getState().open;
  return open.filter((id) => before.devices[id] && !after.devices[id]);
}

/**
 * The app shell's top in the viewport: 0, or below what sits above it (the web's "Open on
 * desktop" band).
 */
const shellTop = () => Math.max(0, Math.round(document.querySelector<HTMLElement>(".eth-shell")?.getBoundingClientRect().top ?? 0));

/**
 * Where a window without a remembered position opens: cascaded from the top center of the app
 * shell, below its top bar (never over the transport).
 */
export function cascade(stack: number, width: number): { x: number; y: number } {
  const step = (stack % 8) * CASCADE;
  return { x: Math.max(INSET, Math.round((window.innerWidth - width) / 2) + step), y: shellTop() + 4 * INSET + step };
}

function DeviceWindow({ device, stack }: { device: Device; stack: number }) {
  const t = useT();
  const trackName = useProjectStore((s) => s.project?.tracks[device.track]?.name);
  const store = useDeviceWindows.getState;
  const width = deviceWindowWidth(device);
  return (
    <FloatingWindow
      id={deviceWindowId(device.id)}
      label={t("devices.window.label", { name: device.name })}
      closeLabel={t("devices.window.close", { name: device.name })}
      title={
        <>
          <span className="eth-device-window__name eth-user-text">{device.name}</span>
          {trackName && <span className="eth-device-window__track eth-user-text">{trackName}</span>}
        </>
      }
      defaultWidth={width}
      defaultPosition={(w) => cascade(stack, w)}
      onClose={() => store().setOpen(device.id, false)}
      onRaise={() => store().raise(device.id)}
      data-device-window={device.id}
    >
      <ReadOnlySurface>
        <DeviceEditor device={device} />
      </ReadOnlySurface>
    </FloatingWindow>
  );
}

/**
 * A built-in device's own editor: its declared layout (or the generic one), with the sample
 * slot unless the layout's waveform widget carries it. Shown in its window, never inline.
 */
export function DeviceEditor({ device }: { device: Device }) {
  const t = useT();
  const sender = useGestureSender();
  const { descriptor, error } = useDescriptor(device);
  const held = useProjectStore((s) => s.safeMode.includes(device.id));
  const { on } = useDeviceOn(device);
  const picker = useAutoSamplePicker(device);
  return (
    <SamplePickerWindow.Provider value={picker}>
      <div className={clsx("eth-device", "eth-device--stack", "eth-device--window", !on && "eth-device--bypassed")} data-device-editor={device.id}>
        {!hasWaveform(descriptor) && !isDrumRack(device) && <SampleSlot device={device} />}
        {/* A drum rack's editor is its pads and pad chains: its rack-level Volume/Pan params
            duplicate the track's fader (still automatable, just not drawn). */}
        {descriptor && !isDrumRack(device) && <DeviceLayoutView device={device} descriptor={descriptor} sender={sender} />}
        {isDrumRack(device) && <DrumRackEditor rack={device} />}
        {descriptor ? null : (
          <div className="eth-device__body">
            <div className="eth-device__status">
              {held ? t("devices.device.not_loaded_safe_mode") : error ? t("devices.device.descriptor_unavailable") : t("devices.device.loading")}
            </div>
          </div>
        )}
      </div>
    </SamplePickerWindow.Provider>
  );
}

/** After the window's opening animation (the picker anchors to where the slot settles). */
const AUTO_PICK_DELAY_MS = parseFloat(duration.normal);

/**
 * The sample picker of a Sampler's window: opens by itself once, when the window opens on a
 * Sampler without a sample (not when the sample is cleared later).
 */
function useAutoSamplePicker(device: Device) {
  const [open, setOpen] = useState(false);
  const [auto] = useState(() => samplerSample(device) === null);
  useEffect(() => {
    if (!auto) return;
    const t = setTimeout(() => setOpen(true), AUTO_PICK_DELAY_MS);
    return () => clearTimeout(t);
  }, [auto]);
  return useMemo(() => ({ device: device.id, open, setOpen }), [device.id, open]);
}

/** Does the device's layout draw the sample waveform (with its own drop slot)? */
function hasWaveform(descriptor: DeviceDescriptor | null): boolean {
  return !!descriptor?.layout?.sections.some((s) => s.items.some((i) => i.widget.type === "SampleWaveform"));
}
