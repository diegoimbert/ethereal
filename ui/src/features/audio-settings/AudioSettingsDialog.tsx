import { useContext, useEffect, useState } from "react";
import type { AudioConfig, TrackId } from "@/generated";
import { RefreshCw } from "lucide-react";
import { Button, Dialog, IconButton, Meter, Select, Toggle, type SelectOption } from "@/kit";
import { useProjectStore, useTrackMeter } from "@/state";
import { cmd, commandErrorText, isCommandFailed, isTauri, TransportContext, type EngineTransport } from "@/transport";
import { PluginFoldersPanel } from "@/features/plugins";
import { PluginListSettings } from "@/features/plugins/PluginListSettings";
import { SandboxDefaultsSettings } from "@/features/plugins/SandboxDefaultsSettings";
import { useEngineEvent } from "@/features/transport-bar/engine";
import { LaunchSettings } from "@/features/project/LaunchSettings";
import { FileFilterSettings } from "@/features/browser/settings/FileFilterSettings";
import { StorageSettings } from "@/features/storage";
import { NewsSettings } from "@/features/news";
import { AdvancedSettings, DiagnosticsSettings } from "./AdvancedSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { InputSettings, ShortcutSettings } from "./InputSettings";
import { LanguageSettings } from "./LanguageSettings";
import { SettingsPlaceholder } from "./SettingsPlaceholder";
import { bufferChoice } from "./buffer";
import { loadAudioDevices, useAudioSettings, type BuiltinSettingsTab } from "./store";
import { useContributions, type SettingsSectionContribution } from "@/ext";
import { formatFixed, useT, type MessageKey } from "@/i18n";
import "./audioSettings.css";

const NO_INPUT = "";
/** The selected driver's default input (`input_device: "default"`). */
const DEFAULT_INPUT = "default";

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
  return e instanceof Error ? e.message : String(e);
}

/** A category of the settings (the sidebar), and its pages (shown one at a time). */
interface Category {
  id: string;
  label: string;
  pages: Array<{ id: string; label: string }>;
}

/**
 * The app's categories and pages. A page id is what `openSettings(id)` takes (the former tab
 * ids still name the first page of their category).
 */
const CATEGORIES: ReadonlyArray<{ id: string; label: MessageKey; pages: ReadonlyArray<{ id: BuiltinSettingsTab; label: MessageKey }> }> = [
  {
    id: "general",
    label: "settings.tabs.general",
    pages: [
      { id: "general", label: "settings.pages.startup" },
      { id: "library", label: "settings.pages.library" },
    ],
  },
  { id: "appearance", label: "settings.tabs.appearance", pages: [{ id: "appearance", label: "settings.tabs.appearance" }] },
  { id: "audio", label: "settings.tabs.audio", pages: [{ id: "audio", label: "settings.tabs.audio" }] },
  {
    id: "input",
    label: "settings.tabs.input",
    pages: [
      { id: "input", label: "settings.pages.mouse" },
      { id: "shortcuts", label: "settings.pages.shortcuts" },
    ],
  },
  {
    id: "plugins",
    label: "settings.tabs.plugins",
    pages: [
      { id: "plugins", label: "settings.pages.plugin_folders" },
      { id: "plugin-list", label: "settings.pages.plugin_list" },
      { id: "plugin-sandbox", label: "settings.pages.plugin_sandbox" },
    ],
  },
  { id: "privacy", label: "settings.tabs.privacy", pages: [{ id: "privacy", label: "settings.tabs.privacy" }] },
  {
    id: "advanced",
    label: "settings.tabs.advanced",
    pages: [
      { id: "advanced", label: "settings.pages.engine" },
      { id: "diagnostics", label: "settings.pages.diagnostics" },
    ],
  },
];

/** The pages shown without an engine (the others say "no engine connected"). */
const NO_ENGINE_PAGES: ReadonlySet<string> = new Set(["general", "appearance", "input", "shortcuts", "plugin-list", "privacy", "diagnostics"]);

/**
 * Settings: a sidebar of categories (a category with several pages lists them under it when
 * open) and one page at a time:
 * - General > Language & startup: language, startup ("Reopen last project on launch").
 *   General > Library: where the projects are kept (web), the library browser's file rules.
 * - Appearance: theme, interface zoom, translucency (kept on this device; needs no engine).
 * - Audio: driver (host API), output and input devices, sample rate and buffer size, the
 *   engine's status, and an input check (the level of armed, monitored tracks). Every change
 *   applies at once (`Engine::SetAudioConfig`, only the changed field); the engine persists
 *   the settings.
 * - Mouse & keyboard > Mouse & trackpad: wheel and buttons (zoom/scroll sensitivity,
 *   inversion, modifier zooms, middle and side buttons; kept on this device). Keyboard
 *   shortcuts: the shortcuts editor.
 * - Plugins > Folders & scan: the folders scanned for plugins (system folders on/off, the
 *   user's folders with a format filter), Rescan and Full rescan. Plugin list: one format per
 *   plugin (the preferred one) and the hidden plugins (kept on this device). Sandboxing: the
 *   plugins a new instance of which starts sandboxed (kept by the engine host).
 * - Privacy: "check for news" (docs/NEWS.md). Kept on this device; needs no engine.
 * - Advanced > Engine: the engine server (remote engine). Diagnostics: the engine log and
 *   the debug panels.
 * Extensions add categories or pages (`@/ext` "settings.tab": `parent` makes it a page of a
 * category) and sections to any page ("settings.section"). Mounted once by the app shell;
 * open it with `openSettings(page)` or `openAudioSettings()`.
 */
export function AudioSettingsDialog() {
  const t = useT();
  const transport = useContext(TransportContext)?.transport ?? null;
  // Load the devices as soon as the engine is there, so the dialog opens ready.
  useEffect(() => {
    if (transport) void loadAudioDevices(transport);
  }, [transport]);
  const open = useAudioSettings((s) => s.open);
  const reason = useAudioSettings((s) => s.reason);
  const tab = useAudioSettings((s) => s.tab);
  const setTab = useAudioSettings((s) => s.setTab);
  const close = useAudioSettings((s) => s.close);
  const extraTabs = useContributions("settings.tab");
  const sections = useContributions("settings.section");
  const categories: Category[] = CATEGORIES.map((c) => ({ id: c.id, label: t(c.label), pages: c.pages.map((p) => ({ id: p.id, label: t(p.label) })) }));
  // Extensions: a page under its category (`parent`), else a category of its own.
  for (const x of extraTabs) {
    const parent = x.parent && categories.find((c) => c.id === x.parent);
    if (parent) {
      parent.pages.push({ id: x.id, label: x.label() });
      continue;
    }
    const at = categories.findIndex((c) => c.id === (x.before ?? "advanced"));
    categories.splice(at < 0 ? categories.length : at, 0, { id: x.id, label: x.label(), pages: [{ id: x.id, label: (x.pageLabel ?? x.label)() }] });
  }
  // An unknown page (an extension gone): the first one.
  const category = categories.find((c) => c.pages.some((p) => p.id === tab)) ?? categories[0]!;
  const page = category.pages.find((p) => p.id === tab) ?? category.pages[0]!;
  const extra = extraTabs.find((x) => x.id === page.id);
  const noEngine = NO_ENGINE_PAGES.has(page.id) || !!extra?.noEngine || sections.some((x) => x.tab === page.id && x.noEngine);
  return (
    <Dialog
      open={open}
      onClose={close}
      title={t("settings.title")}
      className="eth-audio-settings"
      footer={
        <Button onClick={close} data-tour="settings-done">
          {t("common.done")}
        </Button>
      }
    >
      {open && (
        <div className="eth-settings">
          <nav className="eth-settings__nav" aria-label={t("settings.title")}>
            {categories.map((c) => {
              const current = c.id === category.id;
              return (
                <div key={c.id} className="eth-settings__nav-group">
                  <button
                    type="button"
                    className="eth-settings__nav-item"
                    aria-current={current && c.pages.length === 1 ? "page" : undefined}
                    data-current={current || undefined}
                    data-tour={`settings-tab-${c.id}`}
                    onClick={() => setTab(c.pages[0]!.id)}
                  >
                    {c.label}
                  </button>
                  {/* Always there, animated open / closed (its height); closed, out of the
                      tab order. */}
                  {c.pages.length > 1 && (
                    <div className="eth-settings__nav-pages" data-open={current || undefined} inert={!current}>
                      <div className="eth-settings__nav-pages-inner">
                        {c.pages.map((p) => (
                          <button
                            key={p.id}
                            type="button"
                            className="eth-settings__nav-item eth-settings__nav-page"
                            aria-current={p.id === page.id ? "page" : undefined}
                            data-current={p.id === page.id || undefined}
                            onClick={() => setTab(p.id)}
                          >
                            {p.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </nav>
          <div className="eth-settings__panel" role="region" aria-label={page.label}>
            {!transport && !noEngine && <p className="eth-audio-settings__note">{t("settings.no_engine")}</p>}
            <Sections at="start" tab={page.id} sections={sections} transport={transport} />
            {page.id === "general" && (
              <div className="eth-settings__stack">
                <LanguageSettings />
                <LaunchSettings />
              </div>
            )}
            {page.id === "library" && (
              <div className="eth-settings__stack">
                {/* The browser keeps the projects; the desktop app's are files. */}
                {!isTauri() && <StorageSettings />}
                {transport && <FileFilterSettings transport={transport} />}
              </div>
            )}
            {page.id === "appearance" && <AppearanceSettings />}
            {page.id === "input" && <InputSettings />}
            {page.id === "shortcuts" && <ShortcutSettings />}
            {page.id === "privacy" && (
              <div className="eth-settings__stack">
                <NewsSettings />
              </div>
            )}
            {transport && page.id === "audio" && <AudioSettingsBody transport={transport} reason={reason} />}
            {transport && page.id === "plugins" && <PluginFoldersPanel transport={transport} />}
            {page.id === "plugin-list" && <PluginListSettings />}
            {transport && page.id === "plugin-sandbox" && <SandboxDefaultsSettings transport={transport} />}
            {transport && page.id === "advanced" && <AdvancedSettings transport={transport} />}
            {page.id === "diagnostics" && <DiagnosticsSettings />}
            {extra?.Component && (transport || extra.noEngine) && <extra.Component transport={transport} />}
            <Sections at="end" tab={page.id} sections={sections} transport={transport} />
          </div>
        </div>
      )}
    </Dialog>
  );
}

/** The extensions' sections of `tab` at its `at` end. */
function Sections({
  at,
  tab,
  sections,
  transport,
}: {
  at: "start" | "end";
  tab: string;
  sections: ReadonlyArray<SettingsSectionContribution>;
  transport: EngineTransport | null;
}) {
  const shown = sections.filter((x) => x.tab === tab && (x.at ?? "end") === at && (transport || x.noEngine));
  if (shown.length === 0) return null;
  return (
    <div className="eth-settings__stack">
      {shown.map((x, i) => (
        <x.Component key={i} transport={transport} />
      ))}
    </div>
  );
}

function AudioSettingsBody({ transport, reason }: { transport: EngineTransport; reason: "input" | null }) {
  const t = useT();
  // Loaded when the app connected (see AudioSettingsDialog), so this opens without a flicker.
  const devices = useAudioSettings((s) => s.devices);
  const status = useAudioSettings((s) => s.status);
  const unavailable = useAudioSettings((s) => s.unavailable);
  const loading = useAudioSettings((s) => s.loading);
  const allDevices = useAudioSettings((s) => s.allDevices);
  const [error, setError] = useState<string | null>(null);
  const refresh = () => void loadAudioDevices(transport);
  // The host reports a status of its own when it closes or reopens the device (the local
  // engine switched off or on, "Play with").
  useEngineEvent((event) => {
    if (event.type === "Engine" && event.event.type === "Status") useAudioSettings.setState({ status: event.event.status });
  });

  /** Apply one field (the engine keeps the others), then refresh the lists and status. */
  const apply = async (patch: Partial<AudioConfig>) => {
    const config: AudioConfig = {
      backend: null,
      host: null,
      output_device: null,
      input_device: null,
      sample_rate: null,
      buffer_size: null,
      ...patch,
    };
    try {
      await transport.send(cmd("Engine", { type: "SetAudioConfig", config }));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
    await loadAudioDevices(transport);
  };

  if (unavailable) {
    return (
      <p className="eth-audio-settings__note" data-tour="audio-unavailable">
        {t("settings.audio.unavailable", { reason: unavailable })}
      </p>
    );
  }
  // Loading: placeholder rows where the devices go (no jump when they come).
  if (!devices) return loading ? <SettingsPlaceholder rows={4} /> : <p className="eth-audio-settings__note">{t("settings.audio.no_devices")}</p>;

  const cur = devices.current;
  const defaultOutput = devices.outputs.find((d) => d.is_default)?.name ?? devices.outputs[0]?.name ?? "";
  const output = cur.output_device ?? defaultOutput;
  const outputInfo = devices.outputs.find((d) => d.name === output);
  const rates = [...new Set([...(outputInfo?.sample_rates ?? []), ...(status ? [status.sample_rate] : [])])].sort((a, b) => a - b);
  const rate = cur.sample_rate ?? status?.sample_rate ?? null;
  const buffer = bufferChoice(cur.buffer_size, status?.buffer_size, outputInfo?.buffer_size_range ?? null, devices.buffer_size_settable);
  const opts = (values: ReadonlyArray<string>): SelectOption<string>[] => values.map((v) => ({ value: v, label: v, userText: true }));

  return (
    <div className="eth-audio-settings__body">
      {reason === "input" && !cur.input_device && (
        <p className="eth-audio-settings__hint" role="status">
          {t("settings.audio.choose_input_hint")}
        </p>
      )}
      {error && (
        <p className="eth-audio-settings__error" role="alert">
          {error}
        </p>
      )}

      <div className="eth-audio-settings__toolbar">
        {/* Linux lists every ALSA device; the engine keeps the sound server and the hardware. */}
        {(devices.hidden_devices > 0 || allDevices) && (
          <Toggle
            size="sm"
            checked={allDevices}
            label={t("settings.audio.all_devices")}
            onChange={(all) => {
              useAudioSettings.setState({ allDevices: all });
              refresh();
            }}
          />
        )}
        <IconButton
          size="sm"
          tone="ghost"
          label={t("settings.audio.refresh")}
          icon={<RefreshCw className={loading ? "eth-audio-settings__spin" : undefined} />}
          disabled={loading}
          onClick={refresh}
        />
      </div>
      <div className="eth-audio-settings__grid">
        {devices.hosts.length > 1 && (
          <Field label={t("settings.audio.driver")} tour="audio-driver">
            <Select
              aria-label={t("settings.audio.driver")}
              value={cur.host ?? devices.hosts[0]!}
              options={opts(devices.hosts)}
              onChange={(host) => void apply({ host })}
            />
          </Field>
        )}
        <Field label={t("settings.audio.output")} tour="audio-output">
          <Select
            aria-label={t("settings.audio.output")}
            value={output}
            options={devices.outputs.map((d) => ({
              value: d.name,
              label: d.is_default ? t("settings.audio.system_default", { name: d.name }) : d.name,
              userText: true,
            }))}
            onChange={(output_device) => void apply({ output_device })}
          />
        </Field>
        <Field label={t("settings.audio.input")} tour="audio-input">
          <Select
            aria-label={t("settings.audio.input")}
            value={cur.input_device ?? NO_INPUT}
            options={[
              { value: NO_INPUT, label: t("settings.audio.input_none") },
              // Set when the driver changes (device names belong to a driver): the new
              // driver's default input (on ASIO, the output's driver).
              ...(cur.input_device === DEFAULT_INPUT && !devices.inputs.some((d) => d.name === DEFAULT_INPUT)
                ? [{ value: DEFAULT_INPUT, label: t("settings.audio.input_default") }]
                : []),
              ...devices.inputs.map((d) => ({
                value: d.name,
                label: t("settings.audio.input_channels", { name: d.name, channels: d.channels }),
                userText: true,
              })),
            ]}
            onChange={(input_device) => void apply({ input_device })}
          />
        </Field>
        <Field label={t("settings.audio.sample_rate")} tour="audio-sample-rate">
          <Select
            aria-label={t("settings.audio.sample_rate")}
            value={rate !== null ? String(rate) : ""}
            placeholder={t("common.default")}
            options={rates.map((r) => ({ value: String(r), label: t("settings.audio.rate_hz", { rate: r }) }))}
            onChange={(v) => void apply({ sample_rate: Number(v) })}
          />
        </Field>
        <Field label={t("settings.audio.buffer_size")} tour="audio-buffer">
          <Select
            aria-label={t("settings.audio.buffer_size")}
            value={buffer.value !== null ? String(buffer.value) : ""}
            placeholder={t("common.default")}
            disabled={!devices.buffer_size_settable}
            options={buffer.options.map((b) => ({ value: String(b), label: t("settings.audio.buffer_samples", { count: b }) }))}
            onChange={(v) => void apply({ buffer_size: Number(v) })}
          />
        </Field>
      </div>
      {buffer.note && (
        <p className="eth-audio-settings__note" data-testid="buffer-note">
          {buffer.note}
        </p>
      )}

      {status && (
        <p className="eth-audio-settings__status" data-testid="audio-status" data-tour="audio-status">
          {status.device_released ? (
            <>{t("settings.audio.status_released")} · </>
          ) : (
            <>
              {status.running ? t("settings.audio.status_running") : t("settings.audio.status_stopped")} ·{" "}
              <span className="eth-user-text">{status.backend}</span> ·{" "}
            </>
          )}
          {t("settings.audio.status_line", {
            rate: status.sample_rate,
            buffer: status.buffer_size,
            output: formatFixed((status.output_latency / status.sample_rate) * 1000, 1),
            input: formatFixed((status.input_latency / status.sample_rate) * 1000, 1),
          })}
          {status.xruns > 0 ? ` · ${t("settings.audio.dropouts", { count: status.xruns })}` : ""}
        </p>
      )}

      <InputCheck hasInput={!!cur.input_device} />
    </div>
  );
}

/** A labelled row (a plain container: the Select trigger is a button, which a <label> would click twice). */
function Field({ label, tour, children }: { label: string; tour?: string; children: React.ReactNode }) {
  return (
    <div className="eth-audio-settings__field" data-tour={tour}>
      <span className="eth-audio-settings__label">{label}</span>
      {children}
    </div>
  );
}

/** Levels of the armed tracks (with monitoring on, they carry the live input). */
function InputCheck({ hasInput }: { hasInput: boolean }) {
  const t = useT();
  const armed = useProjectStore((s) => s.armedTracks);
  const tracks = useProjectStore((s) => s.project?.tracks);
  return (
    <section className="eth-audio-settings__check" aria-label={t("settings.audio.input_check")} data-tour="audio-input-check">
      <h3 className="eth-audio-settings__heading">{t("settings.audio.input_check")}</h3>
      {!hasInput ? (
        <p className="eth-audio-settings__note">{t("settings.audio.input_check_choose")}</p>
      ) : armed.length === 0 ? (
        <p className="eth-audio-settings__note">{t("settings.audio.input_check_arm")}</p>
      ) : (
        armed.map((id) => <ArmedMeter key={id} track={id} name={tracks?.[id]?.name ?? t("settings.audio.track_fallback")} />)
      )}
    </section>
  );
}

function ArmedMeter({ track, name }: { track: TrackId; name: string }) {
  const m = useTrackMeter(track);
  return (
    <div className="eth-audio-settings__meter">
      <span className="eth-audio-settings__meter-name eth-user-text">{name}</span>
      <Meter levels={m?.peak ?? [0, 0]} className="eth-audio-settings__meter-bar" />
    </div>
  );
}
