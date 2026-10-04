import { Toggle } from "@/kit";
import { useLaunchPrefs } from "./launch";

/** Settings > General: what happens on launch (base-131). */
export function LaunchSettings() {
  const reopen = useLaunchPrefs((s) => s.reopenLast);
  const setReopen = useLaunchPrefs((s) => s.setReopenLast);
  return (
    <div className="eth-settings__stack" data-testid="settings-general">
      <section className="eth-settings__section" aria-label="Startup">
        <h3 className="eth-settings__heading">Startup</h3>
        <Toggle size="sm" checked={reopen} onChange={setReopen} label="Reopen last project on launch" />
        <p className="eth-settings__hint">
          Off: Ethereal starts on the project screen and loads no plugins until you pick a project. On: the last project reopens, but only if Ethereal
          closed properly last time; after a crash you choose.
        </p>
      </section>
    </div>
  );
}
