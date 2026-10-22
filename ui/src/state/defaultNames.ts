/**
 * The names the engine gives new tracks, rack chains, markers and clips (mirrors
 * `ether-controller`: `doc/tracks.rs` `default_name`, `racks` "chain N", `clip_editing`
 * "marker N", clips their sample's name), in the UI's language like the engine's
 * (`name.*`, `Settings::SetLanguage`). Renaming one to an empty name brings this back.
 */
import type { Clip, Marker, Project, RackChain, Track } from "@/generated";
import { inEnglish, t } from "@/i18n";
import { tracksOrdered } from "./selectors";

const byOrder = <T extends { order: string; id: string }>(a: T, b: T) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1);

/** "3 midi", "1 audio", "2 group", "1 vca", "b return" (by its place among tracks of its kind). */
export function defaultTrackName(project: Project, track: Pick<Track, "id" | "kind">): string {
  const sameKind = tracksOrdered(project).filter((t) => t.kind === track.kind);
  const n = Math.max(1, sameKind.findIndex((t) => t.id === track.id) + 1);
  switch (track.kind) {
    case "Audio":
      return t("name.track.audio", { n });
    case "Midi":
      return t("name.track.midi", { n });
    case "Group":
      return t("name.track.group", { n });
    case "Vca":
      return t("name.track.vca", { n });
    case "Return":
      return t("name.track.return", { letter: String.fromCharCode(97 + ((n - 1) % 26)) });
    case "Master":
      return t("name.track.master");
  }
}

/** "chain 2" (by its place in its rack). */
export function defaultChainName(project: Project, chain: Pick<RackChain, "id" | "rack">): string {
  const chains = Object.values(project.rack_chains)
    .filter((c) => c.rack === chain.rack)
    .sort(byOrder);
  return t("name.rack.chain", { n: Math.max(1, chains.findIndex((c) => c.id === chain.id) + 1) });
}

/** Is `name` a rack chain's default name ("chain 3"), in the UI's language or in English? */
export function isDefaultChainName(name: string): boolean {
  const forms = [t("name.rack.chain", { n: 0 }), inEnglish(() => t("name.rack.chain", { n: 0 }))];
  const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return forms.some((f) => new RegExp(`^${escape(f).replace("0", "\\d+")}$`, "i").test(name));
}

/** "marker 3" (by its place on the timeline). */
export function defaultMarkerName(project: Project, marker: Pick<Marker, "id">): string {
  const markers = Object.values(project.markers).sort((a, b) => a.position - b.position || (a.id < b.id ? -1 : 1));
  return t("name.marker", { n: Math.max(1, markers.findIndex((m) => m.id === marker.id) + 1) });
}

/** An audio clip: its sample's name (without the extension); a MIDI clip: none (shown as "MIDI clip"). */
/**
 * The name a clip shows: its own, else its track's. New clips have no name of their own, so
 * they read as their track until renamed.
 */
export function clipDisplayName(project: Pick<Project, "tracks">, clip: Pick<Clip, "name" | "track">): string {
  return clip.name || project.tracks[clip.track]?.name || "";
}

export function defaultClipName(project: Project, clip: Pick<Clip, "content">): string {
  if (clip.content.type !== "Audio") return "";
  const media = project.media[clip.content.media];
  return media ? media.name.replace(/\.[^./\\]+$/, "") : "";
}
