import { useContext, useEffect, useState } from "react";
import type { AudioConfig, TrackId } from "@/generated";
import { RefreshCw } from "lucide-react";
import { Button, Dialog, IconButton, Meter, Select, Tabs, Toggle, type SelectOption } from "@/kit";
import { useProjectStore, useTrackMeter } from "@/state";
import { cmd, commandErrorText, isCommandFailed, isTauri, TransportContext, type EngineTransport } from "@/transport";
import { PluginFoldersPanel } from "@/features/plugins";
import { useEngineEvent } from "@/features/transport-bar/engine";
import { LaunchSettings } from "@/features/project/LaunchSettings";
import { FileFilterSettings } from "@/features/browser/settings/FileFilterSettings";
import { StorageSettings } from "@/features/storage";
import { NewsSettings } from "@/features/news";
import { AdvancedSettings } from "./AdvancedSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { InputSettings } from "./InputSettings";
import { LanguageSettings } from "./LanguageSettings";
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

const TABS: ReadonlyArray<{ id: BuiltinSettingsTab; label: MessageKey }> = [
  { id: "general", label: "settings.tabs.general" },
  { id: "appearance", label: "settings.tabs.appearance" },
  { id: "audio", label: "settings.tabs.audio" },
  { id: "input", label: "settings.tabs.input" },
  { id: "plugins", label: "settings.tabs.plugins" },
  { id: "privacy", label: "settings.tabs.privacy" },
  { id: "advanced", label: "settings.tabs.advanced" },
];

/** The tabs shown without an engine (the others say "no engine connected"). */
const NO_ENGINE_TABS: ReadonlySet<string> = new Set(["general", "appearance", "input", "privacy"]);

/**
 * Settings, in tabs:
 * - Appearance: theme, interface zoom, translucency (kept on this device; needs no engine).
 * - General: startup ("Reopen last project on launch"); library files (
 *   which files the library browser lists, regex rules; needs the engine).
 * - Audio: driver (host API), output and input devices, sample rate and buffer size, the
 *   engine's status, and an input check (the level of armed, monitored tracks). Every change
 *   applies at once (`Engine::SetAudioConfig`, only the changed field); the engine persists
 *   the settings.
 * - Input: mouse wheel and buttons (zoom/scroll sensitivity, inversion, which
 *   modifier zooms, middle and side buttons). Kept on this device; needs no engine.
 * - Plugins: the folders scanned for plugins (system folders on/off, the user's
 *   folders with a format filter), Rescan and Full rescan.
 * - Privacy: "check for news" (docs/NEWS.md). Kept on this device; needs no engine.
 * - Advanced: the engine server (remote engine), the engine log and the debug panels.
 * Extensions add tabs (`@/ext` "settings.tab") and sections to any tab ("settings.section").
 * Mounted once by the app shell; open it with `openSettings(tab)` or `openAudioSettings()`.
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
  const tabs: Array<{ id: string; label: string; tour: string }> = TABS.map((tab) => ({ id: tab.id, label: t(tab.label), tour: `settings-tab-${tab.id}` }));
  for (const x of extraTabs) {
    const at = tabs.findIndex((tab) => tab.id === (x.before ?? "advanced"));
    tabs.splice(at < 0 ? tabs.length : at, 0, { id: x.id, label: x.label(), tour: `settings-tab-${x.id}` });
  }
  const extra = extraTabs.find((x) => x.id === tab);
  const noEngine = NO_ENGINE_TABS.has(tab) || !!extra?.noEngine;
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
          <Tabs label={t("settings.title")} value={tab} onChange={setTab} items={tabs} className="eth-settings__tabs" />
          <div className="eth-settings__panel" role="tabpanel" aria-label={tabs.find((x) => x.id === tab)?.label}>
            {!transport && !noEngine && <p className="eth-audio-settings__note">{t("settings.no_engine")}</p>}
            <Sections at="start" tab={tab} sections={sections} transport={transport} />
            {tab === "general" && (
              <div className="eth-settings__stack">
                <LanguageSettings />
                <LaunchSettings />
                {/* The browser keeps the projects; the desktop app's are files. */}
                {!isTauri() && <StorageSettings />}
                {transport && <FileFilterSettings transport={transport} />}
              </div>
            )}
            {tab === "appearance" && <AppearanceSettings />}
            {tab === "input" && <InputSettings />}
            {tab === "privacy" && (
              <div className="eth-settings__stack">
                <NewsSettings />
              </div>
            )}
            {transport && tab === "audio" && <AudioSettingsBody transport={transport} reason={reason} />}
            {transport && tab === "plugins" && <PluginFoldersPanel transport={transport} />}
            {transport && tab === "advanced" && <AdvancedSettings transport={transport} />}
            {extra && (transport || extra.noEngine) && <extra.Component transport={transport} />}
            <Sections at="end" tab={tab} sections={sections} transport={transport} />
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
  if (!devices) return <p className="eth-audio-settings__note">{loading ? t("settings.audio.loading") : t("settings.audio.no_devices")}</p>;

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
