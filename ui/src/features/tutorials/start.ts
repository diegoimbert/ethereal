// Starting a tour from the welcome screen: a project of its own (kept in the user's
// projects), what the host can do, then the tour.
import { uniqueName } from "@/features/project/projectNames";
import { t } from "@/i18n";
import { useProjectStore } from "@/state";
import { cmd, isCommandFailed, newProjectId, type EngineTransport } from "@/transport";
import { startTour, useTour, type StartOptions } from "./store";
import type { TourDef, TourEnv } from "./types";

/** What this host can do (the record tour branches on it). */
export async function detectEnv(transport: EngineTransport | null): Promise<TourEnv> {
  const env: TourEnv = { host: transport?.kind ?? null, canRecord: false, devicesListable: false };
  if (!transport) return env;
  try {
    env.canRecord = (await transport.send(cmd("Recording", { type: "ListInputs" }))).type === "Inputs";
  } catch (e) {
    // Any other failure (no device right now) still lets the record steps show.
    env.canRecord = !(isCommandFailed(e) && e.error.code === "Unsupported");
  }
  try {
    env.devicesListable = (await transport.send(cmd("Engine", { type: "ListAudioDevices" }))).type === "AudioDevices";
  } catch {
    env.devicesListable = false;
  }
  return env;
}

function waitFor(ok: () => boolean, timeoutMs: number): Promise<boolean> {
  if (ok()) return Promise.resolve(true);
  return new Promise((resolve) => {
    const t0 = Date.now();
    const unsub = useProjectStore.subscribe(() => {
      if (ok()) done(true);
    });
    const timer = setInterval(() => {
      if (ok()) done(true);
      else if (Date.now() - t0 > timeoutMs) done(false);
    }, 50);
    const done = (v: boolean) => {
      unsub();
      clearInterval(timer);
      resolve(v);
    };
  });
}

/** Whether a tour can start here (an overlay is mounted, none is running). */
export function canStartTutorial(): boolean {
  const s = useTour.getState();
  return s.hosts > 0 && s.tour === null;
}

/**
 * Make the tour's project ("jam tutorial", a number when taken), open it, then start the
 * tour. Rejects if the project can't be made.
 */
export async function startTutorial(tour: TourDef, transport: EngineTransport, opts: StartOptions = {}): Promise<void> {
  const id = newProjectId();
  const name = uniqueName(t(tour.projectName), useProjectStore.getState().projects);
  if (tour.project === "empty") await transport.send(cmd("Project", { type: "Create", id, name }));
  else await transport.send(cmd("Template", { type: "NewProject", id, name, template: tour.project === "starter" ? null : tour.project.template }));
  await waitFor(() => useProjectStore.getState().project?.id === id, 5000);
  const env = await detectEnv(transport);
  startTour(tour, { transport, env }, opts);
}
