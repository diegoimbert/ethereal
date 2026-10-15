// "Record audio": the sound device, an audio track, its input, arming, levels and a first take.
// Beginner level: no word of jargon without its meaning. The browser build can't record
// (no audio inputs there): its steps say so and the record steps are skipped.
import { useAudioSettings } from "@/features/audio-settings";
import type { MessageKey } from "@/i18n";
import { useProjectStore } from "@/state";
import { closeFloating } from "../guard";
import type { TourContext, TourDef } from "../types";
import { InputsDiagram, InterfaceDiagram, LevelDiagram } from "./diagrams";
import { Li, P } from "./text";

const project = () => useProjectStore.getState().project;
const audioTracks = () => Object.values(project()?.tracks ?? {}).filter((t) => t.kind === "Audio");
const armedAudio = () => {
  const armed = useProjectStore.getState().armedTracks;
  return audioTracks().filter((t) => armed.includes(t.id));
};
const recording = () => useProjectStore.getState().transport?.recording ?? false;
const settings = () => useAudioSettings.getState();
const inputsPanel = () => document.querySelector('[data-tour="rec-inputs-panel"]') !== null;
const headerOf = (id: string) => document.querySelector(`[data-tour="track-header"][data-track="${id}"]`);

const devices = (ctx: TourContext) => ctx.env.devicesListable;
const records = (ctx: TourContext) => ctx.env.canRecord;
const browser = (ctx: TourContext) => ctx.env.host === "wasm";

/** The usual drivers of this platform, for the "driver" line. */
function driverNote(): MessageKey {
  const p = typeof navigator === "undefined" ? "" : navigator.platform.toLowerCase();
  if (p.startsWith("win")) return "tutorials.record.driver_windows";
  if (p.startsWith("mac")) return "tutorials.record.driver_mac";
  return "tutorials.record.driver_linux";
}

export const RECORD_AUDIO_TOUR: TourDef = {
  id: "record-audio",
  platforms: ["desktop", "web"],
  version: 1,
  title: "tutorials.record.title",
  projectName: "tutorials.record.project_name",
  project: "empty",
  cleanup: () => {
    if (settings().open) settings().close();
    closeFloating();
  },
  steps: [
    {
      id: "intro",
      title: "tutorials.record.intro.title",
      body: (ctx) => (
        <>
          <P k="tutorials.record.intro.p1" />
          {browser(ctx) && <P k="tutorials.record.intro.browser" />}
        </>
      ),
    },
    {
      id: "interface",
      title: "tutorials.record.interface.title",
      illustration: <InterfaceDiagram />,
      body: (
        <>
          <P k="tutorials.record.interface.p1" />
          <P k="tutorials.record.interface.p2" />
        </>
      ),
    },
    {
      id: "inputs-outputs",
      title: "tutorials.record.io.title",
      illustration: <InputsDiagram />,
      body: (
        <ul>
          <Li k="tutorials.record.io.in_out" />
          <Li k="tutorials.record.io.numbers" />
          <Li k="tutorials.record.io.mono_stereo" />
        </ul>
      ),
    },
    {
      id: "open-settings",
      targets: ["settings-button"],
      title: "tutorials.record.open_settings.title",
      body: <P k="tutorials.record.open_settings.body" />,
      hint: "tutorials.record.open_settings.hint",
      until: () => settings().open,
    },
    {
      id: "audio-tab",
      targets: ["settings-tab-audio"],
      title: "tutorials.record.audio_tab.title",
      body: <P k="tutorials.record.audio_tab.body" />,
      hint: "tutorials.record.audio_tab.hint",
      until: () => settings().open && settings().tab === "audio",
    },
    {
      id: "browser-devices",
      when: (ctx) => !devices(ctx),
      targets: ["audio-unavailable"],
      title: "tutorials.record.browser_devices.title",
      body: (
        <>
          <P k="tutorials.record.browser_devices.p1" />
          <P k="tutorials.record.browser_devices.p2" />
        </>
      ),
      fallback: <P k="tutorials.record.browser_devices.fallback" />,
    },
    {
      id: "output",
      when: devices,
      targets: ["audio-driver", "audio-output"],
      title: "tutorials.record.output.title",
      // A function: the platform's driver line is picked when shown.
      body: () => (
        <>
          <P k="tutorials.record.output.body" />
          <P k={driverNote()} />
        </>
      ),
      interactive: true,
    },
    {
      id: "input",
      when: devices,
      targets: ["audio-input"],
      title: "tutorials.record.input.title",
      body: (
        <>
          <P k="tutorials.record.input.p1" />
          <P k="tutorials.record.input.p2" />
        </>
      ),
      interactive: true,
    },
    {
      id: "sample-rate",
      when: devices,
      targets: ["audio-sample-rate"],
      title: "tutorials.record.sample_rate.title",
      body: <P k="tutorials.record.sample_rate.body" />,
      interactive: true,
    },
    {
      id: "buffer",
      when: devices,
      targets: ["audio-buffer", "audio-status"],
      title: "tutorials.record.buffer.title",
      body: (
        <>
          <P k="tutorials.record.buffer.p1" />
          <P k="tutorials.record.buffer.p2" />
        </>
      ),
      interactive: true,
    },
    {
      id: "input-check",
      when: devices,
      targets: ["audio-input-check"],
      title: "tutorials.record.input_check.title",
      body: <P k="tutorials.record.input_check.body" />,
    },
    {
      id: "close-settings",
      targets: ["settings-done"],
      title: "tutorials.record.close_settings.title",
      body: <P k="tutorials.record.close_settings.body" />,
      hint: "tutorials.record.close_settings.hint",
      until: () => !settings().open,
    },
    {
      id: "add-track",
      targets: ["add-track"],
      title: "tutorials.record.add_track.title",
      body: <P k="tutorials.record.add_track.body" />,
      hint: "tutorials.record.add_track.hint",
      until: () => audioTracks().length > 0,
    },
    {
      id: "inputs",
      targets: ["rec-inputs"],
      title: "tutorials.record.inputs.title",
      body: <P k="tutorials.record.inputs.body" />,
      hint: "tutorials.record.inputs.hint",
      until: inputsPanel,
      onEnter: () => {
        if (settings().open) settings().close();
      },
    },
    {
      id: "arm",
      targets: ["rec-inputs-panel"],
      title: "tutorials.record.arm.title",
      body: (ctx) => (
        <>
          <ul>
            <Li k="tutorials.record.arm.input" />
            <Li k="tutorials.record.arm.monitor" />
            <Li k="tutorials.record.arm.headphones" />
          </ul>
          <P k={browser(ctx) ? "tutorials.record.arm.arming_browser" : "tutorials.record.arm.arming"} />
        </>
      ),
      hint: "tutorials.record.arm.hint",
      until: () => armedAudio().length > 0,
      fallback: <P k="tutorials.record.arm.fallback" />,
    },
    {
      id: "level",
      when: records,
      targets: () => {
        const panel = document.querySelector('[data-tour="rec-inputs-panel"]');
        const headers = armedAudio()
          .map((t) => headerOf(t.id))
          .filter((el): el is Element => el !== null);
        return panel ? [...headers, panel] : headers;
      },
      title: "tutorials.record.level.title",
      illustration: <LevelDiagram />,
      body: (
        <>
          <P k="tutorials.record.level.p1" />
          <P k="tutorials.record.level.p2" />
        </>
      ),
    },
    {
      id: "record",
      when: records,
      targets: ["record-button"],
      title: "tutorials.record.record.title",
      body: <P k="tutorials.record.record.body" />,
      hint: "tutorials.record.record.hint",
      until: recording,
      onEnter: () => closeFloating(),
    },
    {
      id: "stop",
      when: records,
      targets: ["record-button"],
      title: "tutorials.record.stop.title",
      body: <P k="tutorials.record.stop.body" />,
      hint: "tutorials.record.stop.hint",
      until: () => !recording(),
      transportKeys: true,
    },
    {
      id: "take",
      when: records,
      targets: () =>
        armedAudio()
          .map((t) => headerOf(t.id))
          .filter((el): el is Element => el !== null),
      title: "tutorials.record.take.title",
      body: (
        <>
          <P k="tutorials.record.take.p1" />
          <P k="tutorials.record.take.p2" />
        </>
      ),
      transportKeys: true,
    },
    {
      id: "browser-record",
      when: (ctx) => !records(ctx),
      targets: ["record-button"],
      title: "tutorials.record.browser_record.title",
      body: <P k="tutorials.record.browser_record.body" />,
      onEnter: () => closeFloating(),
    },
    {
      id: "done",
      title: "tutorials.record.done.title",
      body: <P k="tutorials.record.done.body" />,
    },
  ],
};
