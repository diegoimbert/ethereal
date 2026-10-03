import { useContext, useEffect, useState } from "react";
import type { AudioConfig, TrackId } from "@/generated";
import { RefreshCw } from "lucide-react";
import { Button, Dialog, IconButton, Meter, Select, Tabs, type SelectOption } from "@/kit";
import { useProjectStore, useTrackMeter } from "@/state";
import { cmd, isCommandFailed, TransportContext, type EngineTransport } from "@/transport";
import { PluginFoldersPanel } from "@/features/plugins";
import { SharingSettings } from "@/features/share/SharingSettings";
import { AdvancedSettings } from "./AdvancedSettings";
import { InputSettings } from "./InputSettings";
import { loadAudioDevices, useAudioSettings, type SettingsTab } from "./store";
import "./audioSettings.css";

const BUFFER_SIZES = [32, 64, 128, 256, 512, 1024, 2048];
const NO_INPUT = "";

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message || e.error.code;
  return e instanceof Error ? e.message : String(e);
}

const TABS: ReadonlyArray<{ id: SettingsTab; label: string }> = [
  { id: "audio", label: "Audio" },
  { id: "input", label: "Input" },
  { id: "plugins", label: "Plugins" },
  { id: "sharing", label: "Sharing" },
  { id: "advanced", label: "Advanced" },
];

/**
 * Settings, in tabs (base-115, docs/SHARING.md §8.6):
 * - Audio: driver (host API), output and input devices, sample rate and buffer size, the
 *   engine's status, and an input check (the level of armed, monitored tracks). Every change
 *   applies at once (`Engine::SetAudioConfig`, only the changed field); the engine persists
 *   the settings.
 * - Input (base-135): mouse wheel and buttons (zoom/scroll sensitivity, inversion, which
 *   modifier zooms, middle and side buttons). Kept on this device; needs no engine.
 * - Plugins (base-129): the folders scanned for plugins (system folders on/off, the user's
 *   folders with a format filter), Rescan and Full rescan.
 * - Sharing: identity and sharing preferences. Advanced: sharing servers, engine server,
 *   relay session.
 * Mounted once by the app shell; open it with `openSettings(tab)` or `openAudioSettings()`.
 */
export function AudioSettingsDialog() {
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
  return (
    <Dialog open={open} onClose={close} title="Settings" className="eth-audio-settings" footer={<Button onClick={close}>Done</Button>}>
      {open && (
        <div className="eth-settings">
          <Tabs label="Settings" value={tab} onChange={setTab} items={TABS} className="eth-settings__tabs" />
          <div className="eth-settings__panel" role="tabpanel" aria-label={TABS.find((t) => t.id === tab)?.label}>
            {!transport && tab !== "input" && <p className="eth-audio-settings__note">No engine connected.</p>}
            {tab === "input" && <InputSettings />}
            {transport && tab === "audio" && <AudioSettingsBody transport={transport} reason={reason} />}
            {transport && tab === "plugins" && <PluginFoldersPanel transport={transport} />}
            {transport && tab === "sharing" && <SharingSettings transport={transport} />}
            {transport && tab === "advanced" && <AdvancedSettings transport={transport} />}
          </div>
        </div>
      )}
    </Dialog>
  );
}

function AudioSettingsBody({ transport, reason }: { transport: EngineTransport; reason: "input" | null }) {
  // Loaded when the app connected (see AudioSettingsDialog), so this opens without a flicker.
  const devices = useAudioSettings((s) => s.devices);
  const status = useAudioSettings((s) => s.status);
  const unavailable = useAudioSettings((s) => s.unavailable);
  const loading = useAudioSettings((s) => s.loading);
  const [error, setError] = useState<string | null>(null);
  const refresh = () => void loadAudioDevices(transport);

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
      <p className="eth-audio-settings__note">
        Audio devices can't be changed here ({unavailable}). In the browser, your system chooses the devices; recording needs
        the desktop app.
      </p>
    );
  }
  if (!devices) return <p className="eth-audio-settings__note">{loading ? "Loading devices…" : "No devices."}</p>;

  const cur = devices.current;
  const defaultOutput = devices.outputs.find((d) => d.is_default)?.name ?? devices.outputs[0]?.name ?? "";
  const output = cur.output_device ?? defaultOutput;
  const outputInfo = devices.outputs.find((d) => d.name === output);
  const rates = [...new Set([...(outputInfo?.sample_rates ?? []), ...(status ? [status.sample_rate] : [])])].sort((a, b) => a - b);
  const rate = cur.sample_rate ?? status?.sample_rate ?? null;
  const buffer = cur.buffer_size ?? status?.buffer_size ?? null;
  const opts = (values: ReadonlyArray<string>): SelectOption<string>[] => values.map((v) => ({ value: v, label: v }));

  return (
    <div className="eth-audio-settings__body">
      {reason === "input" && !cur.input_device && (
        <p className="eth-audio-settings__hint" role="status">
          Choose an input device to record from: no input is open yet, so recordings would be silent.
        </p>
      )}
      {error && (
        <p className="eth-audio-settings__error" role="alert">
          {error}
        </p>
      )}

      <div className="eth-audio-settings__toolbar">
        <IconButton
          size="sm"
          tone="ghost"
          label="Refresh devices"
          icon={<RefreshCw className={loading ? "eth-audio-settings__spin" : undefined} />}
          disabled={loading}
          onClick={refresh}
        />
      </div>
      <div className="eth-audio-settings__grid">
        {devices.hosts.length > 1 && (
          <Field label="Driver">
            <Select
              aria-label="Driver"
              value={cur.host ?? devices.hosts[0]!}
              options={opts(devices.hosts)}
              onChange={(host) => void apply({ host })}
            />
          </Field>
        )}
        <Field label="Output device">
          <Select
            aria-label="Output device"
            value={output}
            options={devices.outputs.map((d) => ({ value: d.name, label: d.is_default ? `${d.name} (system default)` : d.name }))}
            onChange={(output_device) => void apply({ output_device })}
          />
        </Field>
        <Field label="Input device">
          <Select
            aria-label="Input device"
            value={cur.input_device ?? NO_INPUT}
            options={[
              { value: NO_INPUT, label: "None (no recording)" },
              ...devices.inputs.map((d) => ({ value: d.name, label: `${d.name} (${d.channels} ch)` })),
            ]}
            onChange={(input_device) => void apply({ input_device })}
          />
        </Field>
        <Field label="Sample rate">
          <Select
            aria-label="Sample rate"
            value={rate !== null ? String(rate) : ""}
            placeholder="Default"
            options={rates.map((r) => ({ value: String(r), label: `${r} Hz` }))}
            onChange={(v) => void apply({ sample_rate: Number(v) })}
          />
        </Field>
        <Field label="Buffer size">
          <Select
            aria-label="Buffer size"
            value={buffer !== null ? String(buffer) : ""}
            placeholder="Default"
            options={BUFFER_SIZES.map((b) => ({ value: String(b), label: `${b} samples` }))}
            onChange={(v) => void apply({ buffer_size: Number(v) })}
          />
        </Field>
      </div>

      {status && (
        <p className="eth-audio-settings__status" data-testid="audio-status">
          {status.running ? "Running" : "Stopped"} · {status.backend} · {status.sample_rate} Hz · {status.buffer_size} samples ·
          latency {((status.output_latency / status.sample_rate) * 1000).toFixed(1)} ms out /{" "}
          {((status.input_latency / status.sample_rate) * 1000).toFixed(1)} ms in
          {status.xruns > 0 ? ` · ${status.xruns} dropouts` : ""}
        </p>
      )}

      <InputCheck hasInput={!!cur.input_device} />
    </div>
  );
}

/** A labelled row (a plain container: the Select trigger is a button, which a <label> would click twice). */
function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="eth-audio-settings__field">
      <span className="eth-audio-settings__label">{label}</span>
      {children}
    </div>
  );
}

/** Levels of the armed tracks (with monitoring on, they carry the live input). */
function InputCheck({ hasInput }: { hasInput: boolean }) {
  const armed = useProjectStore((s) => s.armedTracks);
  const tracks = useProjectStore((s) => s.project?.tracks);
  return (
    <section className="eth-audio-settings__check" aria-label="Input check">
      <h3 className="eth-audio-settings__heading">Input check</h3>
      {!hasInput ? (
        <p className="eth-audio-settings__note">Choose an input device above.</p>
      ) : armed.length === 0 ? (
        <p className="eth-audio-settings__note">Arm a track (● on its header) and turn its monitoring on to see the input level here.</p>
      ) : (
        armed.map((id) => <ArmedMeter key={id} track={id} name={tracks?.[id]?.name ?? "Track"} />)
      )}
    </section>
  );
}

function ArmedMeter({ track, name }: { track: TrackId; name: string }) {
  const m = useTrackMeter(track);
  return (
    <div className="eth-audio-settings__meter">
      <span className="eth-audio-settings__meter-name">{name}</span>
      <Meter levels={m?.peak ?? [0, 0]} className="eth-audio-settings__meter-bar" />
    </div>
  );
}
