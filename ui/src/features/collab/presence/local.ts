// This site's presence v2 fields (docs/COLLAB.md §8.2): the gesture in progress
// (`activity`), the visible part of the arranger (`viewport`) and the peer being followed
// (`following`). A leaf module: the arrangement's gesture code imports `setActivity` from
// here, and `PresenceBar` merges these fields into the published `PresenceState`.
import { create } from "zustand";
import type { Activity, ArrangerViewport, ClipId, Presence, PresenceState, SiteId } from "@/generated";

export interface LocalPresence {
  activity: Activity | null;
  viewport: ArrangerViewport | null;
  following: SiteId | null;
  /** The clip open in this user's piano roll. */
  editingClip: ClipId | null;
}

export const useLocalPresence = create<LocalPresence>()(() => ({ activity: null, viewport: null, following: null, editingClip: null }));

/** The piano roll reports the clip it shows (`null` when it closes). */
export function setEditingClip(editingClip: ClipId | null): void {
  if (useLocalPresence.getState().editingClip !== editingClip) useLocalPresence.setState({ editingClip });
}

/** Set (at the start of a gesture) or clear (`null`, at its end) this user's activity. */
export function setActivity(activity: Activity | null): void {
  useLocalPresence.setState({ activity });
}

/** Follow `site` (`null` stops following). */
export function setFollowing(following: SiteId | null): void {
  if (useLocalPresence.getState().following !== following) useLocalPresence.setState({ following });
}

/** The v2 fields to merge into a `PresenceState` (unset ones are omitted). */
export function presenceV2Fields(): Pick<PresenceState, "activity" | "viewport" | "following" | "editing_clip"> {
  const { activity, viewport, following, editingClip } = useLocalPresence.getState();
  return {
    ...(activity ? { activity } : {}),
    ...(viewport ? { viewport } : {}),
    ...(following ? { following } : {}),
    ...(editingClip ? { editing_clip: editingClip } : {}),
  };
}

/** "dragging", "resizing"… (for "Diego · dragging"). */
export function activityLabel(activity: Activity | null | undefined): string | null {
  if (!activity) return null;
  return activity.kind === "Other" ? "busy" : activity.kind.toLowerCase();
}

/** A site's display name ("you" for this site). */
export function nameOf(site: SiteId, peers: ReadonlyArray<Presence>, me: SiteId | null): string {
  return site === me ? "you" : peers.find((p) => p.site === site)?.name || "someone";
}

/** What a peer is doing, for its chip's tooltip ("Ada · dragging · listening to Bob"). */
export function peerSummary(peer: Presence, peers: ReadonlyArray<Presence>, me: SiteId | null): string {
  const parts = [peer.name || "Anonymous"];
  const activity = activityLabel(peer.state.activity);
  if (activity) parts.push(activity);
  if (peer.state.listening_to) parts.push(`listening to ${nameOf(peer.state.listening_to, peers, me)}`);
  if (peer.state.following) parts.push(`following ${nameOf(peer.state.following, peers, me)}`);
  const followers = peers.filter((q) => q.state.following === peer.site).map((q) => q.name || "Anonymous");
  if (followers.length) parts.push(`followed by ${followers.join(", ")}`);
  return parts.join(" · ");
}
