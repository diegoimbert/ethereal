// Settings > Sharing and the sharing part of Settings > Advanced (docs/SHARING.md §8.6).
import { Plus, X } from "lucide-react";
import { useEffect, useState } from "react";
import type { IceServer } from "@/generated";
import { Button, IconButton, TextInput, Toggle } from "@/kit";
import type { EngineTransport } from "@/transport";
import { IdentitySection } from "./SharePopover";
import { iceUrlError, nativeEngine, pushIce, pushPreferences, pushSignal, signalUrlError, useShareSettings, type ShareSettings } from "./settings";

/** Change a preference and send all three (`Share::SetPreferences`). */
function setPreference(transport: EngineTransport, patch: Partial<Pick<ShareSettings, "resumeOnOpen" | "autoListen" | "relayOnly">>) {
  useShareSettings.getState().update(patch);
  pushPreferences(transport);
}
import "./share.css";

/** Settings > Sharing: identity and the two preferences. */
export function SharingSettings({ transport }: { transport: EngineTransport }) {
  const resume = useShareSettings((s) => s.resumeOnOpen);
  const autoListen = useShareSettings((s) => s.autoListen);
  // The join screen may have saved a name since.
  useEffect(() => useShareSettings.getState().reload(), []);
  return (
    <div className="eth-share-settings" data-testid="settings-sharing">
      <IdentitySection transport={transport} alwaysOpen />
      <section className="eth-share-settings__group" aria-label="Sharing preferences">
        <Toggle size="sm" checked={resume} onChange={(v) => setPreference(transport, { resumeOnOpen: v })} label="Resume sharing when I open a shared project" />
        <p className="eth-share-pop__hint">Links stay valid while a project is closed. Turned off, you share again with the Share button.</p>
        <Toggle
          size="sm"
          checked={autoListen}
          onChange={(v) => setPreference(transport, { autoListen: v })}
          label="Automatically listen to the host when joining with a listen link"
        />
      </section>
    </div>
  );
}

/** Settings > Advanced: signaling server, ICE servers, "Hide my IP". */
export function SharingAdvancedSettings({ transport }: { transport: EngineTransport }) {
  const saved = useShareSettings((s) => s.signalUrl);
  const relayOnly = useShareSettings((s) => s.relayOnly);
  // No TURN client natively: relay-only would fail every connection (p2p-transport #203).
  const native = nativeEngine(transport);
  const update = useShareSettings((s) => s.update);
  const [signal, setSignal] = useState(saved);
  const problem = signalUrlError(signal);
  const applySignal = () => {
    if (problem || signal.trim() === saved) return;
    update({ signalUrl: signal.trim() });
    pushSignal(transport, signal);
  };
  return (
    <section className="eth-share-settings__group" aria-label="Sharing servers" data-testid="settings-sharing-advanced">
      <h3 className="eth-share-settings__heading">Sharing</h3>
      <label className="eth-share-settings__field">
        <span>Signaling server</span>
        <TextInput
          size="sm"
          aria-label="Signaling server"
          placeholder="Default (etherealws.pages.dev)"
          value={signal}
          invalid={!!problem}
          onChange={(e) => setSignal(e.target.value)}
          onBlur={applySignal}
          onKeyDown={(e) => e.key === "Enter" && applySignal()}
        />
      </label>
      {problem ? (
        <p className="eth-share-pop__warn" role="alert">
          {problem}
        </p>
      ) : (
        <p className="eth-share-pop__hint">Introduces people to your computer; it never sees your project. New links carry this address.</p>
      )}
      <IceServersEditor transport={transport} />
      <Toggle
        size="sm"
        checked={relayOnly && !native}
        disabled={native}
        onChange={(v) => setPreference(transport, { relayOnly: v })}
        label="Hide my IP (relay only)"
      />
      <p className="eth-share-pop__hint">
        {native
          ? "Needs a relay (TURN) server: not supported in the desktop app yet."
          : "Connects through a TURN relay only, so others never see your address. Needs a TURN server above."}
      </p>
    </section>
  );
}

const EMPTY: IceServer = { urls: [""], username: null, credential: null };

/** The STUN/TURN list (`Collab::SetIceServers`; empty = the signaling service's own). */
function IceServersEditor({ transport }: { transport: EngineTransport }) {
  const saved = useShareSettings((s) => s.iceServers);
  const update = useShareSettings((s) => s.update);
  const [rows, setRows] = useState<IceServer[]>(saved);
  const commit = (next: IceServer[]) => {
    setRows(next);
    const valid = next.filter((r) => !iceUrlError(r.urls[0] ?? ""));
    if (JSON.stringify(valid) === JSON.stringify(saved)) return;
    update({ iceServers: valid });
    pushIce(transport, valid);
  };
  const edit = (i: number, patch: Partial<{ url: string; username: string; credential: string }>) =>
    setRows(
      rows.map((r, j) =>
        j !== i
          ? r
          : {
              urls: patch.url !== undefined ? [patch.url] : r.urls,
              username: patch.username !== undefined ? patch.username || null : r.username,
              credential: patch.credential !== undefined ? patch.credential || null : r.credential,
            },
      ),
    );
  return (
    <div className="eth-share-ice" data-testid="ice-servers">
      <span className="eth-share-settings__label">ICE servers</span>
      {rows.length === 0 && <p className="eth-share-pop__hint">Default: the sharing service's STUN server.</p>}
      {rows.map((r, i) => {
        const url = r.urls[0] ?? "";
        const bad = url !== "" && !!iceUrlError(url);
        return (
          <div key={i} className="eth-share-ice__row">
            <TextInput
              size="sm"
              aria-label={`ICE server ${i + 1}`}
              placeholder="turn:turn.example.com:3478"
              value={url}
              invalid={bad}
              title={bad ? (iceUrlError(url) ?? undefined) : undefined}
              onChange={(e) => edit(i, { url: e.target.value })}
              onBlur={() => commit(rows)}
            />
            <TextInput
              size="sm"
              aria-label={`ICE server ${i + 1} username`}
              placeholder="Username"
              value={r.username ?? ""}
              onChange={(e) => edit(i, { username: e.target.value })}
              onBlur={() => commit(rows)}
            />
            <TextInput
              size="sm"
              type="password"
              autoComplete="off"
              aria-label={`ICE server ${i + 1} password`}
              placeholder="Password"
              value={r.credential ?? ""}
              onChange={(e) => edit(i, { credential: e.target.value })}
              onBlur={() => commit(rows)}
            />
            <IconButton size="sm" tone="ghost" label={`Remove ICE server ${i + 1}`} icon={<X />} onClick={() => commit(rows.filter((_, j) => j !== i))} />
          </div>
        );
      })}
      <Button size="sm" tone="ghost" onClick={() => setRows([...rows, EMPTY])}>
        <Plus className="eth-share-pop__icon" aria-hidden />
        Add a STUN or TURN server
      </Button>
    </div>
  );
}
