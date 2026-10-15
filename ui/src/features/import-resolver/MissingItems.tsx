/**
 * Where missing items stay reachable after the dialog (owner request: "the list must also be
 * reachable later"): the top bar's "N missing" badge, and a missing device's header badge
 * with its "Resolve…" button. Both open the resolver.
 */
import { AlertTriangle } from "lucide-react";
import type { Device } from "@/generated";
import { Badge, Button, openContextMenu } from "@/kit";
import { useContext } from "react";
import { TransportContext } from "@/transport";
import { useT } from "@/i18n";
import { resolveLabel } from "./menus";
import { useMissingCount } from "./missingCount";
import { openResolver } from "./store";

/** Top bar: "3 missing" while anything is missing; opens the resolver. */
export function MissingItemsBadge() {
  const t = useT();
  const transport = useContext(TransportContext)?.transport;
  const n = useMissingCount();
  if (n === 0) return null;
  return (
    <Button
      size="sm"
      tone="ghost"
      className="eth-missing-badge"
      data-testid="missing-items-badge"
      title={t("resolver.badge.title")}
      onClick={() => void openResolver(undefined, transport)}
    >
      <AlertTriangle aria-hidden className="eth-missing-badge__icon" />
      {t("resolver.badge.count", { count: n })}
    </Button>
  );
}

/**
 * A missing device's header (mounted by `DeviceView` for every device; nothing for the
 * others): what it was, and "Resolve…" (a built-in or plugin replacement).
 */
export function MissingDeviceControls({ device }: { device: Device }) {
  const t = useT();
  if (device.kind.type !== "Missing") return null;
  const m = device.kind.missing;
  const resolve = () => void openResolver({ device: device.id });
  return (
    <span className="eth-plugin-controls eth-missing-device" data-missing-device={device.id}>
      <Badge tone="warn" className="eth-plugin-controls__missing">
        <span
          role="status"
          title={
            m.origin
              ? t("resolver.device.no_equivalent_from", { name: m.description, origin: m.origin })
              : t("resolver.device.no_equivalent", { name: m.description })
          }
          onContextMenu={(e) => openContextMenu(e, [{ label: resolveLabel(), onSelect: resolve }])}
        >
          {t("resolver.device.pass_through")}
        </span>
      </Badge>
      <Button
        size="sm"
        tone="ghost"
        aria-label={t("resolver.device.resolve_named", { name: device.name })}
        title={t("resolver.device.resolve_title")}
        onClick={resolve}
      >
        {resolveLabel()}
      </Button>
    </span>
  );
}
