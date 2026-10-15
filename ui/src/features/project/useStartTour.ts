import { useState } from "react";
import { notify } from "@/features/notifications";
import { errorMessage, type EngineCommands } from "@/features/transport-bar/engine";
import { canStartTutorial, startTutorial, type TourDef } from "@/features/tutorials";
import { useT } from "@/i18n";
import { guardLeave } from "./leaveGuard";
import { useProjectScreen } from "./screenStore";

/**
 * Starting a tour from the welcome screen (the get-started items, the examples' "show me
 * how"): its project is made, the welcome screen hides, and finishing or leaving it comes
 * back here. In a collaboration session it asks first.
 */
export function useStartTour(commands: EngineCommands, onStarted: () => void) {
  const t = useT();
  const [busy, setBusy] = useState(false);
  const transport = commands.transport;
  // Launch isn't done (the engine is loading, #646): the tours wait (shown, disabled).
  const ready = !useProjectScreen((s) => s.launching);
  const start = (tour: TourDef) => {
    if (!transport || !canStartTutorial()) return;
    void guardLeave(async () => {
      setBusy(true);
      try {
        await startTutorial(tour, transport, {
          // Back to the welcome screen, unless the user went on with the last step's action.
          onEnd: (how) => {
            if (how !== "action") useProjectScreen.getState().show("home");
          },
        });
        onStarted();
      } catch (e) {
        notify("Error", t("project.start.failed", { error: errorMessage(e) }));
      } finally {
        setBusy(false);
      }
    }, t("project.leave.start"));
  };
  return { start, busy, ready };
}
