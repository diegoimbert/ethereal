// The example beats (#518, docs/TUTORIALS.md "Example beats"): short loops made only of
// built-in devices and their factory presets, as factory project templates
// (`factory/example-…`, `crates/ether-controller/src/templates/examples.rs`). The welcome
// screen opens one, or walks through it with its tour: the tour makes the example's project,
// plays it, then shows each part (its devices in the inspector, its clip in the editor) and
// says how to make it. Its last step opens the docs guide that rebuilds it from an empty
// project, click by click (`docs-site/content/welcome/example-*.md`).
import { selectTrackEntity } from "@/features/arrangement/actions";
import { openDocs } from "@/features/docs/open";
import { formatList, t, type MessageKey } from "@/i18n";
import { useProjectStore } from "@/state";
import { useEditorStore } from "@/state/editor";
import { useShellStore } from "@/state/shell";
import { cmd } from "@/transport";
import type { TourDef, TourStep } from "../types";
import { P } from "./text";

/** The template's track names (the tour finds the parts by them). */
type PartTrack = "drums" | "bass" | "keys" | "chords";

/** The drum rack's pads of an example, in the order the tour lists them. */
type Drum = "kick" | "snare" | "clap" | "closed_hat" | "open_hat";

const DRUM_NAMES: Readonly<Record<Drum, MessageKey>> = {
  kick: "tutorials.examples.drum.kick",
  snare: "tutorials.examples.drum.snare",
  clap: "tutorials.examples.drum.clap",
  closed_hat: "tutorials.examples.drum.closed_hat",
  open_hat: "tutorials.examples.drum.open_hat",
};

interface Part {
  track: PartTrack;
  /** The devices step: what plays this part and how to set it up. */
  devices: { title: MessageKey; body: MessageKey };
  /** The clip step: the notes or the beat. */
  notes: { title: MessageKey; body: MessageKey };
  /** A drum part's pads. */
  drums?: ReadonlyArray<Drum>;
}

export interface ExampleDef {
  /** `house-beat`: the template is `factory/example-house-beat`, the guide `welcome/example-house-beat`. */
  slug: string;
  template: string;
  title: MessageKey;
  blurb: MessageKey;
  /** The docs page that rebuilds it from an empty project. */
  guide: string;
  tour: TourDef;
}

const project = () => useProjectStore.getState().project;
const trackNamed = (name: PartTrack) => Object.values(project()?.tracks ?? {}).find((tr) => tr.name === name) ?? null;
const clipOf = (name: PartTrack) => {
  const track = trackNamed(name);
  return track ? (Object.values(project()?.clips ?? {}).find((c) => c.track === track.id) ?? null) : null;
};
const playing = () => useProjectStore.getState().transport?.playing ?? false;

/** Select the part's track: the inspector shows its devices (the editor drawer closes to make room). */
function showDevices(name: PartTrack): void {
  const track = trackNamed(name);
  if (!track) return;
  useShellStore.getState().setOpen("bottom", false);
  selectTrackEntity(track.id);
  useShellStore.getState().setCollapsed("right", false);
}

/** Open the part's clip in the editor drawer (the beat editor for the drums). */
function showClip(name: PartTrack): void {
  const clip = clipOf(name);
  if (!clip) return;
  const shell = useShellStore.getState();
  shell.setOpen("right", false);
  useEditorStore.getState().openClip(clip.id);
  shell.openDrawer("piano-roll");
  shell.setCollapsed("bottom", false);
}

function partSteps(part: Part): TourStep[] {
  const drums = part.drums;
  return [
    {
      id: `${part.track}-devices`,
      targets: ["inspector"],
      title: part.devices.title,
      body: drums ? () => <P k={part.devices.body} values={{ sounds: formatList(drums.map((d) => t(DRUM_NAMES[d]))) }} /> : <P k={part.devices.body} />,
      onEnter: () => showDevices(part.track),
      transportKeys: true,
    },
    {
      id: `${part.track}-clip`,
      targets: ["editor"],
      title: part.notes.title,
      body: <P k={part.notes.body} />,
      onEnter: () => showClip(part.track),
      transportKeys: true,
    },
  ];
}

function exampleTour(o: { slug: string; title: MessageKey; intro: MessageKey; bpm: number; bars: number; parts: ReadonlyArray<Part> }): TourDef {
  const guide = `welcome/example-${o.slug}`;
  return {
    id: `example-${o.slug}`,
    platforms: ["desktop", "web"],
    version: 1,
    title: o.title,
    projectName: o.title,
    project: { template: `factory/example-${o.slug}` },
    cleanup: ({ transport }) => {
      if (playing()) void transport?.send(cmd("Transport", { type: "Stop" })).catch(() => undefined);
      useShellStore.getState().setOpen("bottom", false);
    },
    steps: [
      {
        id: "intro",
        title: o.title,
        body: (
          <>
            <P k={o.intro} />
            <P k="tutorials.examples.intro_how" />
          </>
        ),
      },
      {
        id: "play",
        targets: ["play"],
        title: "tutorials.examples.play.title",
        body: <P k="tutorials.examples.play.body" />,
        hint: "tutorials.examples.play.hint",
        until: playing,
        transportKeys: true,
      },
      {
        id: "tempo",
        title: "tutorials.examples.tempo.title",
        body: <P k="tutorials.examples.tempo.body" values={{ bpm: o.bpm, bars: o.bars }} />,
        transportKeys: true,
      },
      {
        id: "tracks",
        // The first part's header (two adjacent rings would hide each other's edge).
        targets: () => [...document.querySelectorAll(`[data-tour="track-header"][data-track="${trackNamed(o.parts[0]!.track)?.id}"]`)],
        title: "tutorials.examples.tracks.title",
        body: <P k="tutorials.examples.tracks.body" />,
        transportKeys: true,
      },
      ...o.parts.flatMap(partSteps),
      {
        id: "done",
        title: "tutorials.examples.done.title",
        body: (
          <>
            <P k="tutorials.examples.done.p1" />
            <P k="tutorials.examples.done.p2" />
          </>
        ),
        transportKeys: true,
        actions: [{ id: "open-guide", label: "tutorials.examples.done.open_guide", run: () => openDocs(guide) }],
      },
    ],
  };
}

const DRUMS_DEVICES = { title: "tutorials.examples.drums.title", body: "tutorials.examples.drums.body" } as const;

export const HOUSE_TOUR = exampleTour({
  slug: "house-beat",
  title: "tutorials.examples.house.title",
  intro: "tutorials.examples.house.intro",
  bpm: 124,
  bars: 2,
  parts: [
    {
      track: "drums",
      drums: ["kick", "clap", "closed_hat", "open_hat"],
      devices: DRUMS_DEVICES,
      notes: { title: "tutorials.examples.beat.title", body: "tutorials.examples.house.beat" },
    },
    {
      track: "bass",
      devices: { title: "tutorials.examples.bass.title", body: "tutorials.examples.house.bass" },
      notes: { title: "tutorials.examples.bass_notes.title", body: "tutorials.examples.house.bass_notes" },
    },
  ],
});

export const HIP_HOP_TOUR = exampleTour({
  slug: "hip-hop-loop",
  title: "tutorials.examples.hip_hop.title",
  intro: "tutorials.examples.hip_hop.intro",
  bpm: 90,
  bars: 2,
  parts: [
    {
      track: "drums",
      drums: ["kick", "snare", "closed_hat"],
      devices: DRUMS_DEVICES,
      notes: { title: "tutorials.examples.beat.title", body: "tutorials.examples.hip_hop.beat" },
    },
    {
      track: "keys",
      devices: { title: "tutorials.examples.keys.title", body: "tutorials.examples.hip_hop.keys" },
      notes: { title: "tutorials.examples.chords.title", body: "tutorials.examples.hip_hop.chords" },
    },
  ],
});

export const LOFI_TOUR = exampleTour({
  slug: "lofi-chords",
  title: "tutorials.examples.lofi.title",
  intro: "tutorials.examples.lofi.intro",
  bpm: 80,
  bars: 4,
  parts: [
    {
      track: "chords",
      devices: { title: "tutorials.examples.keys.title", body: "tutorials.examples.lofi.keys" },
      notes: { title: "tutorials.examples.chords.title", body: "tutorials.examples.lofi.chords" },
    },
    {
      track: "drums",
      drums: ["kick", "snare", "closed_hat"],
      devices: DRUMS_DEVICES,
      notes: { title: "tutorials.examples.beat.title", body: "tutorials.examples.lofi.beat" },
    },
  ],
});

export const BASSLINE_TOUR = exampleTour({
  slug: "simple-bassline",
  title: "tutorials.examples.bassline.title",
  intro: "tutorials.examples.bassline.intro",
  bpm: 120,
  bars: 2,
  parts: [
    {
      track: "drums",
      drums: ["kick", "closed_hat"],
      devices: DRUMS_DEVICES,
      notes: { title: "tutorials.examples.beat.title", body: "tutorials.examples.bassline.beat" },
    },
    {
      track: "bass",
      devices: { title: "tutorials.examples.bass.title", body: "tutorials.examples.bassline.bass" },
      notes: { title: "tutorials.examples.bass_notes.title", body: "tutorials.examples.bassline.bass_notes" },
    },
  ],
});

const example = (slug: string, title: MessageKey, blurb: MessageKey, tour: TourDef): ExampleDef => ({
  slug,
  template: `factory/example-${slug}`,
  title,
  blurb,
  guide: `welcome/example-${slug}`,
  tour,
});

/** The example beats, in the welcome screen's order. */
export const EXAMPLES: ReadonlyArray<ExampleDef> = [
  example("house-beat", "tutorials.examples.house.title", "tutorials.examples.house.blurb", HOUSE_TOUR),
  example("hip-hop-loop", "tutorials.examples.hip_hop.title", "tutorials.examples.hip_hop.blurb", HIP_HOP_TOUR),
  example("lofi-chords", "tutorials.examples.lofi.title", "tutorials.examples.lofi.blurb", LOFI_TOUR),
  example("simple-bassline", "tutorials.examples.bassline.title", "tutorials.examples.bassline.blurb", BASSLINE_TOUR),
];
