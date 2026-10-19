import clsx from "clsx";
import { Box, FileAudio, FolderSearch, Plug, Sparkles } from "lucide-react";
import { useMemo, useState } from "react";
import type {
  DeviceCandidate,
  DeviceDescriptor,
  DeviceId,
  MatchRule,
  MediaCandidate,
  MissingRole,
  PluginCandidate,
  PluginDescriptor,
  PluginFormat,
} from "@/generated";
import { DevicePicker } from "@/features/devices/DevicePicker";
import { Badge, Button, Dialog, Select, Toggle, type SelectOption } from "@/kit";
import { isOffered } from "@/features/devices/chainUtils";
import { useBuiltinTypes } from "@/features/devices/descriptors";
import { useProjectStore } from "@/state";
import { hasFolderPicker, pickRelinkSource, sourceLabel } from "@/features/media-refs";
import type { EngineTransport } from "@/transport";
import { missingSetup, useAiSettings } from "@/features/ai-chat/settings";
import { formatList, Trans, useT, t, type MessageKey } from "@/i18n";
import { aiProvider, buildAiPayload, hasAiConsent } from "./ai";
import {
  aiItems,
  applyResolution,
  askAi,
  cancelAi,
  choose,
  chooseFile,
  chooseInstalled,
  chooseReplacement,
  closeResolver,
  declineAi,
  resolutions,
  searchMedia,
  setApply,
  setAutoShow,
  showAll,
  useResolver,
  visibleKeys,
  type DeviceItem,
  type MediaItem,
  type PluginItem,
  reasonText,
} from "./store";

const FORMAT_LABEL: Record<PluginFormat, string> = { Clap: "clap", Vst3: "vst3", Vst2: "vst2", Au: "au" };

const RULE_LABEL: Record<MatchRule, MessageKey> = {
  SameId: "resolver.rule.same_id",
  SameName: "resolver.rule.same_name",
  SameFamily: "resolver.rule.same_family",
  Similar: "resolver.rule.similar",
  Ai: "resolver.rule.ai",
  Manual: "resolver.rule.manual",
};

const pct = (c: number) => `${Math.round(c * 100)}%`;

function Confidence({ value, ai }: { value: number; ai?: boolean }) {
  const tone = ai ? "accent" : value >= 0.8 ? "ok" : value >= 0.6 ? "warn" : "danger";
  return (
    <Badge tone={tone} className="eth-resolver__confidence">
      {pct(value)}
    </Badge>
  );
}

// A candidate's reason is the engine's English (no message key yet), except the user's own picks.

/** "on Keys, Pads" / "on Keys, Pads and 3 more tracks": where the devices are. */
function useTrackNames(devices: ReadonlyArray<DeviceId>): string {
  return useProjectStore((s) => {
    const p = s.project;
    if (!p) return "";
    const names: string[] = [];
    for (const id of devices) {
      const d = p.devices[id];
      const name = d ? p.tracks[d.track]?.name : undefined;
      if (name && !names.includes(name)) names.push(name);
    }
    if (names.length === 0) return "";
    const shown = names.slice(0, 3).join(", ");
    return names.length > 3 ? t("resolver.where_more", { names: shown, count: names.length - 3 }) : t("resolver.where", { names: shown });
  });
}

const ROLE_LABEL: Record<MissingRole, MessageKey> = {
  Instrument: "resolver.role.instrument",
  AudioEffect: "resolver.role.audio_effect",
  NoteEffect: "resolver.role.note_effect",
};

function candidateLabel(c: PluginCandidate): string {
  const v = c.version ? ` ${c.version}` : "";
  return `${c.name} · ${FORMAT_LABEL[c.format]}${v}${c.vendor ? ` · ${c.vendor}` : ""}`;
}

function PluginRow({ item, installed, focused }: { item: PluginItem; installed: PluginDescriptor[]; focused: boolean }) {
  const t = useT();
  const m = item.missing;
  const chosen = item.choice !== null ? item.candidates[item.choice] : undefined;
  const options: SelectOption<string>[] = [
    ...item.candidates.map((c, i) => ({
      value: String(i),
      label: `${c.rule === "Ai" ? "✦ " : ""}${candidateLabel(c)} (${pct(c.confidence)})`,
      userText: true,
    })),
    { value: "leave", label: t("resolver.leave_missing") },
  ];
  const devices = m.devices.length;
  const where = useTrackNames(m.devices);
  return (
    <li className={clsx("eth-resolver__row", focused && "eth-resolver__row--focused")} data-testid="resolver-plugin" data-key={item.key}>
      <Toggle
        checked={item.apply}
        disabled={item.choice === null}
        onChange={(on) => setApply(item.key, on)}
        aria-label={t("resolver.apply_replacement", { name: m.name })}
        size="sm"
      />
      <Plug className="eth-resolver__icon" aria-hidden />
      <div className="eth-resolver__info">
        <span className="eth-resolver__name eth-user-text">{m.name || m.plugin_id}</span>
        <span className="eth-resolver__meta">
          <span className="eth-user-text">{[m.vendor, FORMAT_LABEL[m.format], m.version].filter(Boolean).join(" · ")}</span>
          {devices > 1 ? ` · ${t("resolver.used_times", { count: devices })}` : ""}
          {where && (
            <>
              {" "}
              <span className="eth-user-text">{where}</span>
            </>
          )}
          {m.params.length > 0 ? ` · ${t("resolver.saved_params", { count: m.params.length })}` : ""}
        </span>
      </div>
      <div className="eth-resolver__choice">
        <Select
          size="sm"
          aria-label={t("resolver.replacement_for", { name: m.name })}
          options={options}
          value={item.choice === null ? "leave" : String(item.choice)}
          onChange={(v) => choose(item.key, v === "leave" ? null : Number(v))}
        />
        {installed.length > 0 && (
          <DevicePicker
            aria-label={t("resolver.choose_plugin_for", { name: m.name })}
            placeholder
            pluginsOnly
            plugins={installed}
            onBeforeSettings={closeResolver}
            onPick={(pick) => {
              const d = pick.device;
              const p = d.type === "Plugin" ? installed.find((i) => i.id === d.plugin_id && i.format === d.format) : undefined;
              if (p) chooseInstalled(item.key, p);
            }}
          >
            {t("resolver.choose_another")}
          </DevicePicker>
        )}
      </div>
      <div className="eth-resolver__why">
        {chosen ? (
          <>
            <Badge tone={chosen.rule === "Ai" ? "accent" : "default"}>
              {chosen.rule === "Ai" && <Sparkles aria-hidden className="eth-resolver__badge-icon" />}
              {t(RULE_LABEL[chosen.rule])}
            </Badge>
            <Confidence value={chosen.confidence} ai={chosen.rule === "Ai"} />
            <span className="eth-resolver__reason" data-i18n-exempt>
              {reasonText(chosen.reason)}
            </span>
            {chosen.plan.type === "KeepState" && <span className="eth-resolver__keep">{t("resolver.state_kept")}</span>}
            {chosen.lost.length > 0 && (
              <details className="eth-resolver__lost">
                <summary>
                  {chosen.lost.length === 1 ? (
                    <Trans k="resolver.lost_one" values={{ name: <span className="eth-user-text">{chosen.lost[0]}</span> }} />
                  ) : (
                    t("resolver.lost", { count: chosen.lost.length })
                  )}
                </summary>
                <ul>
                  {chosen.lost.map((l) => (
                    <li key={l} className="eth-user-text">
                      {l}
                    </li>
                  ))}
                </ul>
              </details>
            )}
            {chosen.rule === "Ai" && !item.apply && <span className="eth-resolver__hint">{t("resolver.review_ai")}</span>}
          </>
        ) : (
          <span className="eth-resolver__hint">{item.candidates.length === 0 ? t("resolver.no_plugin_match") : t("resolver.later_plugin")}</span>
        )}
      </div>
    </li>
  );
}

function deviceCandidateLabel(c: DeviceCandidate): string {
  if (c.replacement.type === "Builtin") return `${c.name} · ${t("resolver.builtin")}`;
  const p = c.replacement.plugin;
  return `${c.name} · ${FORMAT_LABEL[p.format]}${p.vendor ? ` · ${p.vendor}` : ""}`;
}

/** A missing device kind (e.g. every "Live's Hybrid Reverb"): a built-in or plugin replacement, or later. */
function DeviceRow({
  item,
  installed,
  builtins,
  focused,
}: {
  item: DeviceItem;
  installed: PluginDescriptor[];
  builtins: DeviceDescriptor[];
  focused: boolean;
}) {
  const t = useT();
  const m = item.missing;
  const chosen = item.choice !== null ? item.candidates[item.choice] : undefined;
  const where = useTrackNames(m.devices);
  const options: SelectOption<string>[] = [
    ...item.candidates.map((c, i) => ({
      value: String(i),
      label: `${deviceCandidateLabel(c)} (${pct(c.confidence)})`,
      userText: c.replacement.type !== "Builtin",
    })),
    { value: "leave", label: t("resolver.later") },
  ];
  // Anything of the same role: built-ins (racks and drum racks have chains, not a device's
  // parameters), then installed plugins.
  const others = useMemo(() => {
    const b = builtins.filter((d) => d.device_type.type === "Builtin" && isOffered(d) && d.category === m.role && !/Rack$/.test(d.device_type.device));
    const p = installed.filter((x) => x.category === m.role);
    return [
      ...b.map((d) => ({ value: `b:${d.device_type.type === "Builtin" ? d.device_type.device : ""}`, label: d.name, group: t("resolver.builtin") })),
      ...p.map((x, i) => ({ value: `p:${i}`, label: `${x.name} · ${FORMAT_LABEL[x.format]}`, userText: true, group: x.vendor || t("resolver.plugins") })),
    ];
  }, [builtins, installed, m.role, t]);
  const pickOther = (v: string) => {
    if (v.startsWith("b:")) {
      const d = builtins.find((x) => x.device_type.type === "Builtin" && x.device_type.device === v.slice(2));
      if (d && d.device_type.type === "Builtin") chooseReplacement(item.key, { type: "Builtin", device: d.device_type.device }, d.name);
      return;
    }
    const p = installed.filter((x) => x.category === m.role)[Number(v.slice(2))];
    if (p) {
      chooseReplacement(
        item.key,
        { type: "Plugin", plugin: { format: p.format, plugin_id: p.id, name: p.name, vendor: p.vendor, version: p.version } },
        p.name,
      );
    }
  };
  const count = m.devices.length;
  const label = m.description;
  return (
    <li className={clsx("eth-resolver__row", focused && "eth-resolver__row--focused")} data-testid="resolver-device" data-key={item.key}>
      <Toggle
        checked={item.apply}
        disabled={item.choice === null}
        onChange={(on) => setApply(item.key, on)}
        aria-label={t("resolver.apply_replacement", { name: label })}
        size="sm"
      />
      <Box className="eth-resolver__icon" aria-hidden />
      <div className="eth-resolver__info">
        <span className="eth-resolver__name eth-user-text">{label}</span>
        <span className="eth-resolver__meta">
          {m.origin && <span className="eth-user-text">{m.origin}</span>}
          {m.origin ? " · " : ""}
          {t(ROLE_LABEL[m.role])}
          {count > 1 ? ` · ${t("resolver.used_times", { count })}` : ""}
          {where && (
            <>
              {" "}
              <span className="eth-user-text">{where}</span>
            </>
          )}
          {m.params.length > 0 ? ` · ${t("resolver.saved_params", { count: m.params.length })}` : ""}
        </span>
      </div>
      <div className="eth-resolver__choice">
        <Select
          size="sm"
          aria-label={t("resolver.replacement_for", { name: label })}
          options={options}
          value={item.choice === null ? "leave" : String(item.choice)}
          onChange={(v) => choose(item.key, v === "leave" ? null : Number(v))}
        />
        {others.length > 0 && (
          <Select
            size="sm"
            searchable
            aria-label={t("resolver.choose_device_for", { name: label })}
            placeholder={t("resolver.choose_another")}
            options={others}
            value=""
            onChange={pickOther}
          />
        )}
      </div>
      <div className="eth-resolver__why">
        {chosen ? (
          <>
            <Confidence value={chosen.confidence} />
            <span className="eth-resolver__reason" data-i18n-exempt>
              {reasonText(chosen.reason)}
            </span>
            <span className="eth-resolver__keep">{t("resolver.device_keep")}</span>
            {!item.apply && <span className="eth-resolver__hint">{t("resolver.switch_on")}</span>}
          </>
        ) : (
          <span className="eth-resolver__hint">{t("resolver.later_device")}</span>
        )}
      </div>
    </li>
  );
}

function mediaLabel(c: MediaCandidate): string {
  return `${c.display} (${pct(c.confidence)})`;
}

function MediaRow({ item, transport, focused, searching }: { item: MediaItem; transport: EngineTransport; focused: boolean; searching: boolean }) {
  const t = useT();
  const m = item.missing;
  const chosen = item.choice !== null ? item.candidates[item.choice] : undefined;
  const options: SelectOption<string>[] = [
    ...item.candidates.map((c, i) => ({ value: String(i), label: mediaLabel(c), userText: true })),
    { value: "leave", label: t("resolver.later") },
  ];
  const pick = async () => {
    try {
      const source = await pickRelinkSource(transport);
      if (source) chooseFile(item.key, source, sourceLabel(source));
    } catch (e) {
      useResolver.setState({ error: e instanceof Error ? e.message : String(e) });
    }
  };
  return (
    <li className={clsx("eth-resolver__row", focused && "eth-resolver__row--focused")} data-testid="resolver-media" data-key={item.key}>
      <Toggle
        checked={item.apply}
        disabled={item.choice === null}
        onChange={(on) => setApply(item.key, on)}
        aria-label={t("resolver.apply_file", { name: m.name })}
        size="sm"
      />
      <FileAudio className="eth-resolver__icon" aria-hidden />
      <div className="eth-resolver__info">
        <span className="eth-resolver__name eth-user-text">{m.name}</span>
        <span className={clsx("eth-resolver__meta", m.path !== null && "eth-user-text")} title={m.path ?? undefined}>
          {m.path ?? t("resolver.in_project")}
        </span>
      </div>
      <div className="eth-resolver__choice">
        <Select
          size="sm"
          aria-label={t("resolver.file_for", { name: m.name })}
          options={options}
          value={item.choice === null ? "leave" : String(item.choice)}
          onChange={(v) => choose(item.key, v === "leave" ? null : Number(v))}
        />
        <Button size="sm" onClick={() => void pick()}>
          {t("resolver.choose_file")}
        </Button>
      </div>
      <div className="eth-resolver__why">
        {chosen ? (
          <>
            <Confidence value={chosen.confidence} />
            <span className="eth-resolver__reason" data-i18n-exempt>
              {reasonText(chosen.reason)}
            </span>
          </>
        ) : (
          <span className="eth-resolver__hint">{!item.searched && searching ? t("resolver.searching") : t("resolver.no_file_match")}</span>
        )}
      </div>
    </li>
  );
}

function AiBar() {
  const t = useT();
  const st = useResolver();
  const { ai, installed } = st;
  const items = aiItems(st);
  const [show, setShow] = useState(false);
  // Re-render when the AI chat gets (or loses) a provider setup.
  useAiSettings((s) => `${s.provider}:${missingSetup(s)}`);
  const provider = aiProvider();
  if (!provider || items.length === 0 || installed.length === 0) return null;
  const payload = buildAiPayload(
    items.map((i) => i.missing),
    installed,
  ).payload;
  const consent = ai.status === "consent";
  return (
    <section className="eth-resolver__ai" aria-label={t("resolver.ai.label")}>
      <div className="eth-resolver__ai-bar">
        <Sparkles className="eth-resolver__icon" aria-hidden />
        <span className="eth-resolver__ai-text">{t("resolver.ai.unsure", { count: items.length, provider: provider.label })}</span>
        {ai.calls > 0 && <Badge>{t("resolver.ai.calls", { count: ai.calls })}</Badge>}
        {ai.status === "running" ? (
          <Button size="sm" onClick={cancelAi}>
            {t("resolver.cancel")}
          </Button>
        ) : (
          !consent && (
            <Button size="sm" tone="accent" onClick={() => void askAi()}>
              {t("resolver.ai.ask")}
            </Button>
          )
        )}
      </div>
      {ai.status === "running" && (
        <p className="eth-resolver__hint" role="status">
          {t("resolver.ai.asking", { provider: provider.label })}
        </p>
      )}
      {ai.message && (
        <p className="eth-resolver__hint" role="status">
          {ai.message}
        </p>
      )}
      {consent && (
        <div className="eth-resolver__consent" role="group" aria-label={t("resolver.ai.consent_label")}>
          <p>{t("resolver.ai.consent", { provider: provider.label })}</p>
          <Button size="sm" tone="ghost" onClick={() => setShow(!show)} aria-expanded={show}>
            {show ? t("resolver.ai.hide_payload") : t("resolver.ai.show_payload")}
          </Button>
          {show && (
            <pre className="eth-resolver__payload" data-testid="resolver-ai-payload">
              {JSON.stringify(payload, null, 2)}
            </pre>
          )}
          <div className="eth-resolver__consent-actions">
            <Button size="sm" onClick={declineAi}>
              {t("resolver.ai.not_now")}
            </Button>
            <Button size="sm" tone="accent" onClick={() => void askAi({ consent: true })}>
              {t("resolver.ai.allow")}
            </Button>
          </div>
        </div>
      )}
      {!consent && hasAiConsent() && ai.calls === 0 && <p className="eth-resolver__hint">{t("resolver.ai.allowed", { provider: provider.label })}</p>}
    </section>
  );
}

/**
 * "Resolve missing items": each missing plugin and audio file with a proposed match, its
 * confidence and reason; accept all, per item, choose another (installed plugins / a file),
 * search a folder, or leave missing. Applying is one undo step.
 */
export function ResolverDialog({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const s = useResolver();
  const shown = visibleKeys(s);
  const builtins = useBuiltinTypes();
  const plugins = s.plugins.filter((p) => shown.has(p.key));
  const media = s.media.filter((m) => shown.has(m.key));
  const devices = s.devices.filter((d) => shown.has(d.key));
  const total = s.plugins.length + s.media.length + s.devices.length;
  const focusedItem = s.focus !== null && shown.size === 1;
  const pending = resolutions(s);
  const n = pending.plugins.length + pending.media.length + pending.devices.length;
  const searching = s.search !== null && !s.search.done;
  const sectioned = s.section !== null && !focusedItem;
  const focusName = focusedItem ? (plugins[0]?.missing.name ?? devices[0]?.missing.description ?? media[0]?.missing.name) : null;
  const acceptAll = () => {
    for (const p of plugins) if (p.choice !== null && p.candidates[p.choice]?.rule !== "Ai") setApply(p.key, true);
    for (const d of devices) if (d.choice !== null) setApply(d.key, true);
    for (const m of media) if (m.choice !== null) setApply(m.key, true);
    void applyResolution(transport);
  };
  const searchFolder = async () => {
    if (!hasFolderPicker(transport)) return;
    const folder = await transport.pickFolder();
    if (folder) await searchMedia(transport, folder);
  };
  return (
    <Dialog
      open={s.open}
      onClose={closeResolver}
      title={
        focusName ? (
          <Trans k="resolver.title_named" values={{ name: <span className="eth-user-text">{focusName}</span> }} />
        ) : s.section === "devices" ? (
          t("resolver.title_devices")
        ) : s.section === "samples" ? (
          t("resolver.title_samples")
        ) : (
          t("resolver.title")
        )
      }
      className="eth-resolver-dialog"
      footer={
        <>
          {s.error && (
            <span className="eth-resolver__error" role="alert">
              {s.error}
            </span>
          )}
          <Toggle size="sm" className="eth-resolver__autoshow" checked={s.autoShow} onChange={(on) => setAutoShow(on)} label={t("resolver.auto_show")} />
          <Button size="sm" onClick={closeResolver}>
            {t("resolver.later")}
          </Button>
          <Button size="sm" disabled={s.applying || total === 0} onClick={acceptAll}>
            {t("resolver.accept_all")}
          </Button>
          <Button size="sm" tone="accent" disabled={s.applying || n === 0} onClick={() => void applyResolution(transport)}>
            {t("resolver.apply", { count: n })}
          </Button>
        </>
      }
    >
      <div className="eth-resolver" data-testid="resolver">
        {s.loading ? (
          <p className="eth-resolver__hint" role="status">
            {t("resolver.loading")}
          </p>
        ) : total === 0 ? (
          <p className="eth-resolver__hint">{t("resolver.nothing_missing")}</p>
        ) : (
          <>
            <p className="eth-resolver__summary">
              {focusedItem || sectioned ? (
                <>
                  <Button size="sm" tone="ghost" onClick={showAll}>
                    {t("resolver.show_all", { count: total })}
                  </Button>
                </>
              ) : (
                t("resolver.summary", {
                  items: formatList(
                    [
                      s.plugins.length > 0 && t("resolver.count.plugins", { count: s.plugins.length }),
                      s.devices.length > 0 && t("resolver.count.devices", { count: s.devices.length }),
                      s.media.length > 0 && t("resolver.count.files", { count: s.media.length }),
                    ].filter((x): x is string => Boolean(x)),
                  ),
                })
              )}
            </p>
            {s.pluginHost && <AiBar />}
            {plugins.length > 0 && (
              <section className="eth-resolver__section" aria-label={t("resolver.section.plugins_label")}>
                <h3 className="eth-resolver__heading">{t("resolver.plugins")}</h3>
                <ul className="eth-resolver__list">
                  {plugins.map((p) => (
                    <PluginRow key={p.key} item={p} installed={s.installed} focused={focusedItem} />
                  ))}
                </ul>
              </section>
            )}
            {devices.length > 0 && (
              <section className="eth-resolver__section" aria-label={t("resolver.section.devices_label")}>
                <h3 className="eth-resolver__heading">{t("resolver.section.devices")}</h3>
                <ul className="eth-resolver__list">
                  {devices.map((d) => (
                    <DeviceRow key={d.key} item={d} installed={s.installed} builtins={builtins} focused={focusedItem} />
                  ))}
                </ul>
              </section>
            )}
            {media.length > 0 && (
              <section className="eth-resolver__section" aria-label={t("resolver.section.files_label")}>
                <h3 className="eth-resolver__heading">
                  {t("resolver.section.files")}
                  <span className="eth-resolver__search" role="status">
                    {searching
                      ? t("resolver.search.running", { count: s.search?.scanned ?? 0 })
                      : s.search?.done
                        ? t("resolver.search.done", { count: s.search.scanned })
                        : ""}
                  </span>
                  {s.folderSearch && hasFolderPicker(transport) && (
                    <Button size="sm" tone="ghost" onClick={() => void searchFolder()}>
                      <FolderSearch aria-hidden className="eth-resolver__badge-icon" />
                      {t("resolver.search.folder")}
                    </Button>
                  )}
                </h3>
                <ul className="eth-resolver__list">
                  {media.map((m) => (
                    <MediaRow key={m.key} item={m} transport={transport} focused={focusedItem} searching={searching} />
                  ))}
                </ul>
              </section>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
}
