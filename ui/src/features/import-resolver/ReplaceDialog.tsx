import { ArrowDown, Blocks, Plug } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { DeviceCategory, DeviceDescriptor, PluginFormat, ReplaceReport } from "@/generated";
import { DevicePicker } from "@/features/devices/DevicePicker";
import { fetchBuiltinTypes } from "@/features/devices/descriptors";
import type { LibraryDevicePayload } from "@/features/devices/libraryDevice";
import { Button, Dialog, Toggle } from "@/kit";
import { useProjectStore } from "@/state";
import type { EngineTransport } from "@/transport";
import { Trans, useT } from "@/i18n";
import { isMissingPlugin } from "./catalog";
import { acceptReplacement, applyReplace, choiceOf, chooseReplacement, closeReplace, isRackDevice, setRetain, useReplace } from "./replace";

const FORMAT_LABEL: Record<PluginFormat, string> = { Clap: "clap", Vst3: "vst3", Vst2: "vst2", Au: "au" };

function Summary({ report, retain }: { report: ReplaceReport; retain: boolean }) {
  const t = useT();
  const lanes = report.lanes_removed;
  if (!retain || report.matched === "Reset") {
    return (
      <p className="eth-resolver__hint" data-testid="replace-summary">
        {lanes > 0 ? t("resolver.replace.reset_lanes", { count: lanes }) : t("resolver.replace.reset")}
      </p>
    );
  }
  const kept = report.kept;
  const match =
    report.matched === "SameIds"
      ? t("resolver.replace.same_ids")
      : report.matched === "ByName"
        ? t("resolver.replace.by_name", { count: kept })
        : report.matched === "ByIndex"
          ? t("resolver.replace.by_index", { count: kept })
          : t("resolver.replace.unmatched");
  return (
    <div className="eth-resolver__why" data-testid="replace-summary">
      <span className={report.matched === "SameIds" || report.lost.length === 0 ? "eth-resolver__keep" : "eth-resolver__reason"}>{match}</span>
      {report.lost.length === 1 && (
        <span className="eth-resolver__lost">
          <Trans k="resolver.replace.cant_keep_one" values={{ name: <span className="eth-user-text">{report.lost[0]}</span> }} />
        </span>
      )}
      {report.lost.length > 1 && (
        <details className="eth-resolver__lost" open={report.lost.length <= 3}>
          <summary>{t("resolver.replace.cant_keep", { count: report.lost.length })}</summary>
          <ul>
            {report.lost.map((l) => (
              <li key={l} className="eth-user-text">
                {l}
              </li>
            ))}
          </ul>
        </details>
      )}
      {lanes > 0 && <span className="eth-resolver__hint">{t("resolver.replace.lanes_removed", { count: lanes })}</span>}
    </div>
  );
}

/**
 * "Replace device…": swap any device but a rack for a factory device or an installed plugin
 * of the same role (instrument, audio effect, MIDI effect), optionally retaining its
 * automation, mappings and parameter values (one undo step). The device keeps its place and
 * on/off state, and its sidechain source when the new device has a sidechain input.
 */
export function ReplaceDialog({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const s = useReplace();
  const device = useProjectStore((st) => (s.device ? st.project?.devices[s.device] : undefined));
  const [builtins, setBuiltins] = useState<DeviceDescriptor[]>([]);
  useEffect(() => {
    if (!s.open) return;
    let alive = true;
    fetchBuiltinTypes(transport).then(
      (l) => alive && setBuiltins(l),
      () => alive && setBuiltins([]),
    );
    return () => {
      alive = false;
    };
  }, [transport, s.open]);
  const plugin = device?.kind.type === "Plugin" ? device.kind.plugin : null;
  // Same role as the device being replaced; any for a plugin that isn't installed (the
  // engine still refuses another role when it knows the plugin's).
  const role = useMemo<DeviceCategory | undefined>(() => {
    if (!device) return undefined;
    if (device.kind.type === "Plugin") {
      const { format, plugin_id } = device.kind.plugin;
      return s.installed?.find((p) => p.format === format && p.id === plugin_id)?.category;
    }
    if (device.kind.type === "Missing") return device.kind.missing.role;
    const type = device.kind.device.type;
    return builtins.find((d) => d.device_type.type === "Builtin" && d.device_type.device === type)?.category;
  }, [device, s.installed, builtins]);
  const accept = useMemo(() => (device ? (p: LibraryDevicePayload) => acceptReplacement(device, p) : undefined), [device]);
  const open = s.open && !!device && !(device.kind.type === "Builtin" && isRackDevice(device.kind.device.type));
  const missing = device ? isMissingPlugin(device, s.installed) : false;
  const meta = plugin
    ? [plugin.vendor, FORMAT_LABEL[plugin.format], plugin.version].filter(Boolean).join(" · ")
    : device?.kind.type === "Missing"
      ? device.kind.missing.origin
      : t("devices.library.source_factory");
  const picked = s.choice;
  return (
    <Dialog
      open={open}
      onClose={closeReplace}
      title={
        device ? (
          <Trans k="resolver.replace.title_named" values={{ name: <span className="eth-user-text">{device.name}</span> }} />
        ) : (
          t("resolver.replace.title")
        )
      }
      className="eth-resolver-dialog eth-replace-dialog"
      footer={
        <>
          {s.error && (
            <span className="eth-resolver__error" role="alert">
              {s.error}
            </span>
          )}
          <Button size="sm" onClick={closeReplace}>
            {t("resolver.cancel")}
          </Button>
          <Button size="sm" tone="accent" disabled={s.applying || !s.choice} onClick={() => void applyReplace(transport)}>
            {t("resolver.replace.replace")}
          </Button>
        </>
      }
    >
      {device && (
        <div className="eth-resolver eth-replace" data-testid="replace-device">
          <div className="eth-replace__plugin">
            {plugin ? <Plug className="eth-resolver__icon" aria-hidden /> : <Blocks className="eth-resolver__icon" aria-hidden />}
            <div className="eth-resolver__info">
              <span className="eth-resolver__name eth-user-text">{plugin ? plugin.name || plugin.plugin_id : device.name}</span>
              <span className="eth-resolver__meta">
                <span className="eth-user-text">{meta}</span>
                {missing ? ` · ${t("resolver.replace.not_installed")}` : ""}
              </span>
            </div>
          </div>
          <ArrowDown className="eth-resolver__icon eth-replace__arrow" aria-hidden />
          {s.loading ? (
            <p className="eth-resolver__hint" role="status">
              {t("resolver.replace.loading")}
            </p>
          ) : (
            <DevicePicker
              aria-label={t("resolver.replace.replacement")}
              placeholder={!picked}
              plugins={s.installed ?? undefined}
              categories={role ? [role] : undefined}
              accept={accept}
              onBeforeSettings={closeReplace}
              onPick={(p) => {
                const c = choiceOf(p, s.installed);
                if (c) void chooseReplacement(c, transport);
              }}
            >
              {picked ? (
                <span className="eth-user-text">
                  {picked.plugin
                    ? `${picked.name} · ${FORMAT_LABEL[picked.plugin.format]}${picked.plugin.version ? ` ${picked.plugin.version}` : ""}`
                    : picked.name}
                </span>
              ) : (
                t("resolver.replace.choose")
              )}
            </DevicePicker>
          )}
          <Toggle size="sm" checked={s.retain} onChange={(on) => void setRetain(on, transport)} label={t("resolver.replace.retain")} />
          <div role="status" aria-live="polite">
            {!s.choice ? (
              <p className="eth-resolver__hint">{t("resolver.replace.keeps")}</p>
            ) : s.report ? (
              <Summary report={s.report} retain={s.retain} />
            ) : s.error ? null : (
              <p className="eth-resolver__hint">{t("resolver.replace.checking")}</p>
            )}
          </div>
        </div>
      )}
    </Dialog>
  );
}
