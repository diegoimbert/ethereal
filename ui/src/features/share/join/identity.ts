/**
 * The sharing identity (name + colour), a UI setting (docs/SHARING.md §8.6, decision 16):
 * stored in `localStorage` and pushed to the engine with `Share::SetIdentity`. The join
 * screen asks for a name ("Join as") only when none is stored yet.
 */
import type { Color } from "@/generated";

/** Storage key (shared with the Settings > Sharing tab). */
export const IDENTITY_KEY = "eth.share.identity";

export interface ShareIdentity {
  name: string;
  color: Color | null;
}

/** Longest name the engine accepts (1-64 characters). */
export const MAX_NAME_CHARS = 64;

export function validName(name: string): boolean {
  const n = name.trim();
  return n.length > 0 && [...n].length <= MAX_NAME_CHARS;
}

export function loadIdentity(): ShareIdentity | null {
  try {
    const v = JSON.parse(localStorage.getItem(IDENTITY_KEY) ?? "null") as Partial<ShareIdentity> | null;
    if (!v || typeof v.name !== "string" || !validName(v.name)) return null;
    return { name: v.name.trim(), color: typeof v.color === "number" ? v.color : null };
  } catch {
    return null;
  }
}

export function saveIdentity(identity: ShareIdentity): void {
  try {
    localStorage.setItem(IDENTITY_KEY, JSON.stringify({ name: identity.name.trim(), color: identity.color }));
  } catch {
    // storage unavailable: asked again next time
  }
}
