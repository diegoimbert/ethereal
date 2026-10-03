// Sharing preferences (docs/SHARING.md §8.6, decision 16): the anonymous identity (name and
// colour) and the Sharing / Advanced settings. They live in UI settings (localStorage) and
// are pushed to the engine at startup and on every change (`Share::SetIdentity`,
// `Share::SetServers`, `Collab::SetIceServers`).
import { create } from "zustand";
import type { Color, IceServer } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";

/**
 * Identity colours offered in the Share popover: the hub's `PEER_COLORS`
 * (`crates/ether-collab/src/wire.rs`, a parity test checks it). Colours are data, like
 * track colours. The host gives another one when it is taken.
 */
export const PEER_COLORS: readonly Color[] = [0xff94a6, 0x5cffe8, 0xffa529, 0xbffb00, 0x92a7ff, 0xd86ce4, 0x25ffa8, 0xf7f47c];

/** Names are 1-64 characters after trimming (`ShareCommand::SetIdentity`). */
export const NAME_MAX = 64;

export interface ShareSettings {
  /** Display name ("" = not chosen yet: the popover shows the identity editor expanded). */
  name: string;
  color: Color | null;
  /** "Resume sharing when I open a shared project" (decision 13, default on). */
  resumeOnOpen: boolean;
  /** "Automatically listen to the host when joining with a listen link" (default on). */
  autoListen: boolean;
  /** Settings > Advanced: signaling service ("" = the default). */
  signalUrl: string;
  /** Settings > Advanced: STUN/TURN servers (empty = the signaling service's). */
  iceServers: IceServer[];
  /** "Hide my IP (relay only)": TURN only. */
  relayOnly: boolean;
}

export const DEFAULT_SHARE_SETTINGS: ShareSettings = {
  name: "",
  color: null,
  resumeOnOpen: true,
  autoListen: true,
  signalUrl: "",
  iceServers: [],
  relayOnly: false,
};

const KEY = "eth-share-settings";

function load(): ShareSettings {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<ShareSettings>;
    return { ...DEFAULT_SHARE_SETTINGS, ...v };
  } catch {
    return { ...DEFAULT_SHARE_SETTINGS };
  }
}

function save(s: ShareSettings) {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // storage unavailable: this session only
  }
}

interface SettingsStore extends ShareSettings {
  update(patch: Partial<ShareSettings>): void;
  /** Re-read localStorage (tests). */
  reload(): void;
}

export const useShareSettings = create<SettingsStore>()((set, get) => ({
  ...load(),
  update: (patch) => {
    set(patch);
    const { update: _u, reload: _r, ...rest } = get();
    save(rest);
  },
  reload: () => set(load()),
}));

/** Why a display name is not accepted (`null` = fine). */
export function nameError(name: string): string | null {
  const n = name.trim();
  if (!n) return "Enter a name.";
  if ([...n].length > NAME_MAX) return `At most ${NAME_MAX} characters.`;
  return null;
}

/** Why a signaling server address is not accepted (`null` = fine; "" = the default). */
export function signalUrlError(url: string): string | null {
  const u = url.trim();
  if (!u) return null;
  return /^https?:\/\/[^\s/]+/.test(u) ? null : "Enter an http(s) address, such as https://signal.example.com.";
}

/** Why an ICE server URL is not accepted (`null` = fine). */
export function iceUrlError(url: string): string | null {
  return /^(stun|stuns|turn|turns):[^\s]+$/.test(url.trim()) ? null : "Use stun:host:port or turn:host:port.";
}

const quiet = (p: Promise<unknown>) => void p.catch(() => undefined);

/** Send the identity (when one is chosen). */
export function pushIdentity(transport: EngineTransport, s: Pick<ShareSettings, "name" | "color"> = useShareSettings.getState()): void {
  if (nameError(s.name)) return;
  quiet(transport.send(cmd("Share", { type: "SetIdentity", name: s.name.trim(), color: s.color })));
}

/** Send the signaling service (a non-default one, or on change). */
export function pushSignal(transport: EngineTransport, signalUrl: string = useShareSettings.getState().signalUrl): void {
  const signal = signalUrl.trim();
  if (signalUrlError(signal)) return;
  quiet(transport.send(cmd("Share", { type: "SetServers", signal_url: signal || null, invite_origin: null })));
}

/** The ICE servers that are valid enough to send. */
export const validIceServers = (servers: ReadonlyArray<IceServer>): IceServer[] =>
  servers.filter((x) => x.urls.length > 0 && x.urls.every((u) => !iceUrlError(u)));

/** Send the ICE servers (`null` = the service's / relay's own). */
export function pushIce(transport: EngineTransport, servers: ReadonlyArray<IceServer> = useShareSettings.getState().iceServers): void {
  const ice = validIceServers(servers);
  quiet(transport.send(cmd("Collab", { type: "SetIceServers", servers: ice.length ? ice : null })));
}

/** At startup (or on a new engine): push what differs from the engine's defaults. */
export function pushShareSettings(transport: EngineTransport): void {
  const s = useShareSettings.getState();
  pushIdentity(transport, s);
  if (s.signalUrl.trim()) pushSignal(transport, s.signalUrl);
  if (validIceServers(s.iceServers).length) pushIce(transport, s.iceServers);
}
