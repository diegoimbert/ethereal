import { ArrowDown, Plug } from "lucide-react";
import { useMemo } from "react";
import type { PluginFormat, ReplaceReport } from "@/generated";
import { DevicePicker } from "@/features/devices/DevicePicker";
import { Button, Dialog, Toggle } from "@/kit";
import { useProjectStore } from "@/state";
import type { EngineTransport } from "@/transport";
import { Trans, useT } from "@/i18n";
import { isMissingPlugin } from "./catalog";
import { applyReplace, chooseReplacement, closeReplace, pluginChoiceKey, setRetain, useReplace } from "./replace";

const FORMAT_LABEL: Record<PluginFormat, string> = { Clap: "CLAP", Vst3: "VST3", Vst2: "VST2", Au: "AU" };

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
 * "Replace plugin…": swap any plugin device's plugin for an installed one, optionally
 * retaining its automation, mappings and parameter values (one undo step). The device keeps
 * its place, sidechain source and on/off state.
 */
export function ReplaceDialog({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const s = useReplace();
  const device = useProjectStore((st) => (s.device ? st.project?.devices[s.device] : undefined));
  const plugin = device?.kind.type === "Plugin" ? device.kind.plugin : null;
  const installed = useMemo(() => s.installed ?? [], [s.installed]);
  const others = useMemo(() => installed.filter((p) => !plugin || pluginChoiceKey(p) !== `${plugin.format}:${plugin.plugin_id}`), [installed, plugin]);
  // Same kind (instrument / effect) as the plugin being replaced; any when it isn't installed.
  const kind = plugin ? installed.find((p) => p.format === plugin.format && p.id === plugin.plugin_id)?.category : undefined;
  const picked = installed.find((p) => pluginChoiceKey(p) === s.choice);
  const open = s.open && !!plugin;
  const missing = device ? isMissingPlugin(device, s.installed) : false;
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
      {plugin && device && (
        <div className="eth-resolver eth-replace" data-testid="replace-plugin">
          <div className="eth-replace__plugin">
            <Plug className="eth-resolver__icon" aria-hidden />
            <div className="eth-resolver__info">
              <span className="eth-resolver__name eth-user-text">{plugin.name || plugin.plugin_id}</span>
              <span className="eth-resolver__meta">
                <span className="eth-user-text">{[plugin.vendor, FORMAT_LABEL[plugin.format], plugin.version].filter(Boolean).join(" · ")}</span>
                {missing ? ` · ${t("resolver.replace.not_installed")}` : ""}
              </span>
            </div>
          </div>
          <ArrowDown className="eth-resolver__icon eth-replace__arrow" aria-hidden />
          {s.installed === null ? (
            <p className="eth-resolver__hint" role="status">
              {t("resolver.replace.loading")}
            </p>
          ) : others.length === 0 ? (
            <p className="eth-resolver__hint">{t("resolver.replace.none_installed")}</p>
          ) : (
            <DevicePicker
              aria-label={t("resolver.replace.replacement")}
              placeholder={!picked}
              pluginsOnly
              plugins={others}
              categories={kind ? [kind] : undefined}
              onBeforeSettings={closeReplace}
              onPick={(p) => p.device.type === "Plugin" && void chooseReplacement(`${p.device.format}:${p.device.plugin_id}`, transport)}
            >
              {picked ? (
                <span className="eth-user-text">{`${picked.name} · ${FORMAT_LABEL[picked.format]}${picked.version ? ` ${picked.version}` : ""}`}</span>
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
