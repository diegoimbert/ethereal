import { Info } from "lucide-react";
import { useMemo } from "react";
import type { Device, DeviceCategory } from "@/generated";
import { DeviceLibrary } from "@/features/devices/DeviceLibrary";
import { useBuiltinTypes } from "@/features/devices/descriptors";
import "@/features/devices/deviceLibrary.css";
import type { LibraryDevicePayload } from "@/features/devices/libraryDevice";
import { Popover, Toggle } from "@/kit";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { useT } from "@/i18n";
import { acceptReplacement, choiceOf, closeReplace, isRackDevice, replaceWith, setRetain, useReplace } from "./replace";

/**
 * "Replace device…" (mounted under every device card's header, `DeviceView`): a popover like
 * the add library: "Retain automations and parameters" at the top (what it keeps: its
 * tooltip, on hover), then the factory devices and installed plugins of the device's role
 * (never a rack, nor the device itself). A row replaces the device at once (one undo step); the device keeps its place, its
 * on/off state and its sidechain source (when the new device has a sidechain input).
 */
export function ReplacePopover({ device }: { device: Device }) {
  const t = useT();
  const open = useReplace((s) => s.open && s.device === device.id);
  if (device.kind.type === "Builtin" && isRackDevice(device.kind.device.type)) return null;
  return (
    <div className="eth-device__replace-anchor">
      <Popover
        open={open}
        onOpenChange={(o) => !o && closeReplace()}
        placement="bottom-start"
        aria-label={t("resolver.replace.title_named", { name: device.name })}
        className="eth-replace-popover"
        trigger={() => null}
      >
        {open && <ReplaceLibrary device={device} />}
      </Popover>
    </div>
  );
}

function ReplaceLibrary({ device }: { device: Device }) {
  const t = useT();
  const transport = useOptionalTransport();
  const installed = useReplace((s) => s.installed);
  const loading = useReplace((s) => s.loading);
  const retain = useReplace((s) => s.retain);
  const builtins = useBuiltinTypes();
  // Same role as the device being replaced; any for a plugin that isn't installed (the
  // engine still refuses another role when it knows the plugin's).
  const role = useMemo<DeviceCategory | undefined>(() => {
    if (device.kind.type === "Plugin") {
      const { format, plugin_id } = device.kind.plugin;
      return installed?.find((p) => p.format === format && p.id === plugin_id)?.category;
    }
    if (device.kind.type === "Missing") return device.kind.missing.role;
    const type = device.kind.device.type;
    return builtins.find((d) => d.device_type.type === "Builtin" && d.device_type.device === type)?.category;
  }, [device, installed, builtins]);
  const accept = useMemo(() => (p: LibraryDevicePayload) => acceptReplacement(device, p), [device]);
  return (
    <div className="eth-replace" data-testid="replace-device">
      {/* What retaining keeps (or not): the tooltip of the toggle and its info icon. */}
      <div className="eth-replace__options" title={retain ? t("resolver.replace.retain_hint") : t("resolver.replace.reset_hint")}>
        <Toggle size="sm" checked={retain} onChange={setRetain} label={t("resolver.replace.retain")} />
        <Info className="eth-replace__info" aria-hidden />
      </div>
      <DeviceLibrary
        categories={role ? [role] : undefined}
        plugins={loading ? [] : (installed ?? undefined)}
        accept={accept}
        onClose={closeReplace}
        onPick={(p) => {
          const c = choiceOf(p, installed);
          if (c) void replaceWith(c, transport);
        }}
      />
    </div>
  );
}
