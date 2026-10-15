import clsx from "clsx";
import { AppWindow } from "lucide-react";
import type { Device } from "@/generated";
import { IconButton } from "@/kit";
import { useDeviceWindow, type DeviceWindowControl } from "./useDeviceWindow";
import { useT } from "@/i18n";

export interface DeviceWindowButtonProps {
  device: Device;
  control: DeviceWindowControl;
  className?: string;
}

/**
 * The "open window" toggle of a device header (base-153): the plugin's editor window, or
 * the factory device's window. Highlighted (pressed) while the window is open.
 */
export function DeviceWindowButton({ device, control, className }: DeviceWindowButtonProps) {
  const t = useT();
  const plugin = device.kind.type === "Plugin";
  return (
    <IconButton
      size="sm"
      tone="ghost"
      className={clsx("eth-device__window-toggle", className)}
      active={control.open}
      data-viewer-allow
      disabled={!control.available}
      label={t("devices.window.open", { name: device.name })}
      title={control.open ? t("devices.window.close_hint") : plugin ? t("devices.window.open_plugin") : t("devices.window.open_device")}
      icon={<AppWindow />}
      onClick={control.toggle}
    />
  );
}

/** `DeviceWindowButton` with its own window state (headers that don't otherwise need it). */
export function DeviceWindowToggle({ device, className }: { device: Device; className?: string }) {
  return <DeviceWindowButton device={device} control={useDeviceWindow(device)} className={className} />;
}
