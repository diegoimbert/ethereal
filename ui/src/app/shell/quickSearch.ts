/**
 * Quick search (⌘F / Ctrl+F, keymap `search.open`): one box that finds the project's tracks,
 * clips, devices and markers by name (here, synchronously) and the library's items (the
 * engine's index, `QuickSearch.tsx`). Enter jumps to the result (`goTo`):
 *
 * - a track: selected (the inspector shows it) and scrolled into view;
 * - a clip: selected and revealed on the timeline (and its track scrolled into view);
 * - a device: its track selected, the device card selected, focused and scrolled to;
 * - a marker: the cursor (and, when stopped, the playhead) moves there, revealed;
 * - a library item: the library panel opens on it (`features/browser/reveal.ts`).
 */
import type { Clip, Device, LibraryItem, Marker, Project, Track } from "@/generated";
import { selectAtLaneClick, selectTrackEntity } from "@/features/arrangement/actions";
import { arrangementView } from "@/features/arrangement/uiStore";
import { revealInBrowser } from "@/features/browser/reveal";
import { useDeviceClipboard } from "@/features/devices/deviceClipboard";
import { setPlayStart } from "@/features/time-edits/marker";
import { t, type MessageKey } from "@/i18n";
import { clipDisplayName, defaultMarkerName, defaultTrackName, tracksOrdered } from "@/state";
import { openLeftTab } from "@/state/shell";
import type { EngineTransport } from "@/transport";
import { fuzzyScore } from "./commands";

/** Most project results listed (best first). */
export const MAX_PROJECT_RESULTS = 60;
/** Library items asked of the engine per query. */
export const LIBRARY_LIMIT = 20;
/** Room kept between a revealed position and the timeline's edge (px). */
const REVEAL_MARGIN_PX = 48;

export type QuickResult =
  | { type: "track"; key: string; name: string; detail: string; track: Track }
  | { type: "clip"; key: string; name: string; detail: string; clip: Clip }
  | { type: "device"; key: string; name: string; detail: string; device: Device }
  | { type: "marker"; key: string; name: string; detail: string; marker: Marker }
  | { type: "library"; key: string; name: string; detail: string; item: LibraryItem };

/** On equal scores: tracks, then devices, clips, markers. */
const KIND_RANK: Record<QuickResult["type"], number> = { track: 0, device: 1, clip: 2, marker: 3, library: 4 };

const PROJECT_KIND: Record<Exclude<QuickResult["type"], "library">, MessageKey> = {
  track: "shell.search.kind_track",
  clip: "shell.search.kind_clip",
  device: "shell.search.kind_device",
  marker: "shell.search.kind_marker",
};

const LIBRARY_KIND: Record<LibraryItem["kind"], MessageKey> = {
  Audio: "shell.search.kind_audio",
  Midi: "shell.search.kind_midi",
  Preset: "shell.search.kind_preset",
  Project: "shell.search.kind_project",
};

/** What a result is, as shown in its row ("track", "sample"...). */
export function resultKindLabel(r: QuickResult): string {
  return t(r.type === "library" ? LIBRARY_KIND[r.item.kind] : PROJECT_KIND[r.type]);
}

const trackName = (project: Project, track: Track) => track.name || defaultTrackName(project, track);

/** The project's entities matching `query`, best first (none for an empty query). */
export function searchProject(project: Project, query: string): QuickResult[] {
  if (!query.trim()) return [];
  const found: { r: QuickResult; s: number }[] = [];
  const add = (r: QuickResult) => {
    const s = fuzzyScore(query, r.name);
    if (s >= 0) found.push({ r, s });
  };
  const tracks = tracksOrdered(project);
  const order = new Map(tracks.map((tr, i) => [tr.id, i]));
  for (const track of tracks) add({ type: "track", key: `track:${track.id}`, name: trackName(project, track), detail: "", track });
  for (const device of Object.values(project.devices)) {
    const track = project.tracks[device.track];
    if (track) add({ type: "device", key: `device:${device.id}`, name: device.name, detail: trackName(project, track), device });
  }
  const clips = Object.values(project.clips).sort((a, b) => (order.get(a.track) ?? 0) - (order.get(b.track) ?? 0) || a.start - b.start);
  for (const clip of clips) {
    const track = project.tracks[clip.track];
    if (track) add({ type: "clip", key: `clip:${clip.id}`, name: clipDisplayName(project, clip), detail: trackName(project, track), clip });
  }
  const markers = Object.values(project.markers).sort((a, b) => a.position - b.position);
  for (const marker of markers)
    add({ type: "marker", key: `marker:${marker.id}`, name: marker.name || defaultMarkerName(project, marker), detail: "", marker });
  // Stable sort: equal scores keep the kind order, then the project's order.
  return found
    .map((x, i) => ({ ...x, i }))
    .sort((a, b) => b.s - a.s || KIND_RANK[a.r.type] - KIND_RANK[b.r.type] || a.i - b.i)
    .slice(0, MAX_PROJECT_RESULTS)
    .map((x) => x.r);
}

/** Library items from the engine as results (in the engine's relevance order). */
export function libraryResults(items: ReadonlyArray<LibraryItem>): QuickResult[] {
  return items.map((item) => ({ type: "library", key: `library:${item.id}`, name: item.name, detail: item.path.split("/").slice(0, -1).join("/"), item }));
}

/** Scroll a track's row of the arrangement into view (after the selection rendered). */
function scrollTrackIntoView(track: string): void {
  requestAnimationFrame(() => {
    const row = document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${CSS.escape(track)}"]`);
    row?.scrollIntoView?.({ block: "nearest" });
  });
}

/** Jump to a result (see the module docs). */
export function goTo(r: QuickResult, transport: EngineTransport | null): void {
  switch (r.type) {
    case "track":
      selectTrackEntity(r.track.id);
      scrollTrackIntoView(r.track.id);
      return;
    case "clip": {
      selectAtLaneClick(r.clip.track, r.clip.id, "replace");
      const view = arrangementView.getState();
      // The clip's end first, then its start: a clip wider than the view shows its start.
      view.reveal(r.clip.start + r.clip.length, REVEAL_MARGIN_PX);
      view.reveal(r.clip.start, REVEAL_MARGIN_PX);
      scrollTrackIntoView(r.clip.track);
      return;
    }
    case "device": {
      selectTrackEntity(r.device.track);
      useDeviceClipboard.getState().select(r.device.id);
      scrollTrackIntoView(r.device.track);
      // The card shows once the inspector renders the track's chain.
      let tries = 0;
      const focusCard = () => {
        const card = document.querySelector<HTMLElement>(`.eth-device[data-device="${CSS.escape(r.device.id)}"]`);
        if (card) {
          card.scrollIntoView?.({ block: "nearest", inline: "nearest" });
          card.focus({ preventScroll: true });
        } else if (++tries < 10) requestAnimationFrame(focusCard);
      };
      requestAnimationFrame(focusCard);
      return;
    }
    case "marker":
      if (transport) setPlayStart(transport, r.marker.position);
      arrangementView.getState().reveal(r.marker.position, REVEAL_MARGIN_PX);
      return;
    case "library":
      openLeftTab("library");
      revealInBrowser(r.item);
      return;
  }
}

export { useQuickSearch } from "./quickSearchStore";
