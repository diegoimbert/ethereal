// Settings > Advanced (docs/SHARING.md §8.6): the sharing servers, the engine server (remote
// engine) and the relay session (the relay-era join form, §9).
import { Button } from "@/kit";
import type { EngineTransport } from "@/transport";
import { useCollabStore } from "@/features/collab/store";
import { RemoteEngineSettings } from "@/features/remote/RemoteEngineSettings";
import { SharingAdvancedSettings } from "@/features/share/SharingSettings";
import { useAudioSettings } from "./store";

export function AdvancedSettings({ transport }: { transport: EngineTransport }) {
  return (
    <div className="eth-settings__stack" data-testid="settings-advanced">
      <SharingAdvancedSettings transport={transport} />
      <RemoteEngineSettings />
      <RelaySessionSettings />
    </div>
  );
}

/** The relay join form stays available here (§9): it opens the relay dialog. */
function RelaySessionSettings() {
  const status = useCollabStore((s) => s.status);
  const open = () => {
    useAudioSettings.getState().close();
    useCollabStore.getState().setDialogOpen(true);
  };
  return (
    <section className="eth-settings__section" aria-label="Relay session" data-testid="settings-relay">
      <h3 className="eth-settings__heading">Relay session</h3>
      <p className="eth-settings__hint">
        For always-on studio servers and networks without internet: everyone joins the same relay (<code>ether-collab-relay</code>) instead of
        the sharer's computer.
      </p>
      {status.type === "Offline" ? (
        <div>
          <Button size="sm" onClick={open}>
            Join a relay session…
          </Button>
        </div>
      ) : (
        <div className="eth-settings__row">
          <span>
            {status.type === "Online" ? "In the relay session" : "Connecting to"} <strong>{status.session}</strong>
          </span>
          <Button size="sm" onClick={open}>
            Open session…
          </Button>
        </div>
      )}
    </section>
  );
}
