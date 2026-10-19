// "Make your first track" (#14): from an empty project to a few notes playing, with only what
// comes with Ethereal (no samples: the instrument is the built-in synth, or the poly synth).
// A MIDI track, its instrument, a clip, notes in the piano roll, play and stop. The last step
// opens the docs guide that goes on from there (`docs-site/content/welcome/first-beat.md`):
// a second instrument, an effect, the mix.
import { useArrangementUi } from "@/features/arrangement/uiStore";
import { useDeviceWindows } from "@/features/devices/window";
import { openDocs } from "@/features/docs/open";
import { useProjectStore } from "@/state";
import { useEditorStore } from "@/state/editor";
import { useShellStore } from "@/state/shell";
import { cmd } from "@/transport";
import type { TourDef } from "../types";
import { Li, P } from "./text";

/** Notes the "draw some notes" step asks for. */
export const FIRST_TRACK_NOTES = 3;

const project = () => useProjectStore.getState().project;
/** The tour's track: the project's first MIDI track (the project starts empty). */
const track = () => Object.values(project()?.tracks ?? {}).find((t) => t.kind === "Midi") ?? null;
const clip = () => {
  const tr = track();
  return tr ? (Object.values(project()?.clips ?? {}).find((c) => c.track === tr.id) ?? null) : null;
};
const noteCount = () => {
  const c = clip();
  return c ? Object.values(project()?.notes ?? {}).filter((n) => n.clip === c.id).length : 0;
};
const lane = () => {
  const tr = track();
  return tr ? [...document.querySelectorAll(`[data-lane="${tr.id}"]`)] : [];
};
/** The clip is open in the piano roll, on screen. */
const clipOpen = () => {
  const c = clip();
  return c !== null && useEditorStore.getState().clip === c.id && document.querySelector('[data-tour="editor"] [data-testid="piano-roll-grid"]') !== null;
};
const playing = () => useProjectStore.getState().transport?.playing ?? false;

export const FIRST_TRACK_TOUR: TourDef = {
  id: "first-track",
  platforms: ["desktop", "web"],
  version: 1,
  title: "project.start.first_track",
  projectName: "tutorials.first_track.project_name",
  project: "empty",
  cleanup: ({ transport }) => {
    if (playing()) void transport?.send(cmd("Transport", { type: "Stop" })).catch(() => undefined);
    useArrangementUi.getState().setInstrumentPick(null);
  },
  steps: [
    {
      id: "intro",
      title: "tutorials.first_track.intro.title",
      body: (
        <>
          <P k="tutorials.first_track.intro.p1" />
          <P k="tutorials.first_track.intro.p2" />
        </>
      ),
    },
    {
      id: "add-track",
      targets: ["add-track"],
      title: "tutorials.first_track.add_track.title",
      body: <P k="tutorials.first_track.add_track.body" />,
      hint: "tutorials.first_track.add_track.hint",
      until: () => track() !== null,
    },
    {
      id: "instrument",
      targets: ["instrument-pick"],
      title: "tutorials.first_track.instrument.title",
      body: (
        <>
          <P k="tutorials.first_track.instrument.p1" />
          <P k="tutorials.first_track.instrument.p2" />
        </>
      ),
      hint: "tutorials.first_track.instrument.hint",
      until: () => useArrangementUi.getState().instrumentPick === null,
      fallback: <P k="tutorials.first_track.instrument.fallback" />,
    },
    {
      // The picked instrument's window opens (once the engine added it).
      id: "instrument-window",
      targets: () => [...document.querySelectorAll("[data-device-window]")],
      title: "tutorials.first_track.window.title",
      body: <P k="tutorials.first_track.window.body" />,
      fallback: <P k="tutorials.first_track.instrument.fallback" />,
    },
    {
      id: "clip",
      targets: lane,
      title: "tutorials.first_track.clip.title",
      body: <P k="tutorials.first_track.clip.body" />,
      hint: "tutorials.first_track.clip.hint",
      until: () => clip() !== null,
      // The picked instrument's window and the editor drawer would hide the lane.
      onEnter: () => {
        const windows = useDeviceWindows.getState();
        windows.closeMany(windows.open);
        useShellStore.getState().setOpen("bottom", false);
      },
    },
    {
      id: "open-clip",
      targets: lane,
      title: "tutorials.first_track.open_clip.title",
      body: <P k="tutorials.first_track.open_clip.body" />,
      hint: "tutorials.first_track.open_clip.hint",
      until: clipOpen,
    },
    {
      id: "notes",
      targets: ["editor"],
      title: "tutorials.first_track.notes.title",
      body: (
        <ul>
          <Li k="tutorials.first_track.notes.add" />
          <Li k="tutorials.first_track.notes.edit" />
        </ul>
      ),
      hint: "tutorials.first_track.notes.hint",
      until: () => noteCount() >= FIRST_TRACK_NOTES,
    },
    {
      id: "play",
      targets: ["play"],
      title: "tutorials.first_track.play.title",
      body: <P k="tutorials.first_track.play.body" />,
      hint: "tutorials.first_track.play.hint",
      until: playing,
      transportKeys: true,
    },
    {
      id: "stop",
      targets: ["play"],
      title: "tutorials.first_track.stop.title",
      body: <P k="tutorials.first_track.stop.body" />,
      hint: "tutorials.first_track.stop.hint",
      until: () => !playing(),
      transportKeys: true,
    },
    {
      id: "done",
      title: "tutorials.first_track.done.title",
      body: (
        <>
          <P k="tutorials.first_track.done.p1" />
          <P k="tutorials.first_track.done.p2" />
        </>
      ),
      actions: [{ id: "open-guide", label: "tutorials.examples.done.open_guide", run: () => openDocs("welcome/first-beat") }],
    },
  ],
};
