import { useCallback, useEffect, useState } from "react";
import type { TemplateInfo, TemplateKind } from "@/generated";
import { useEngineCommands, useEngineEvent } from "@/features/transport-bar/engine";
import { cmd } from "@/transport";
import { message } from "./model";

/**
 * The templates of `kind` (factory, then user), listed while `active` and again whenever
 * the library changes (`TemplateEvent::Changed`, from any client).
 */
export function useTemplates(kind: TemplateKind | null, active: boolean): { templates: TemplateInfo[] | null; error: string | null } {
  const { transport } = useEngineCommands();
  const [templates, setTemplates] = useState<TemplateInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(() => {
    if (!transport) return;
    transport
      .send(cmd("Template", { type: "List", kind }))
      .then((r) => {
        if (r.type === "Templates") {
          setTemplates(r.templates);
          setError(null);
        }
      })
      .catch((e: unknown) => setError(message(e)));
  }, [transport, kind]);
  useEffect(() => {
    if (active) refresh();
  }, [active, refresh]);
  useEngineEvent((e) => {
    if (active && e.type === "Template" && e.event.type === "Changed") refresh();
  });
  return { templates, error };
}
