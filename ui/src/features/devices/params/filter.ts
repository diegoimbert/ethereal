import type { ParamInfo } from "@/generated";
import { foldForSearch, sourceParam } from "@/i18n";

/**
 * Visible params whose name or group contains `query` (case and accents ignored), as shown or
 * as the engine names them (English).
 */
export function filterParams(params: ReadonlyArray<ParamInfo>, query: string): ParamInfo[] {
  const q = foldForSearch(query.trim());
  const hay = (p: ParamInfo) => {
    const s = sourceParam(p);
    return foldForSearch([p.name, p.group ?? "", s.name, s.group ?? ""].join(" "));
  };
  return params.filter((p) => !p.hidden && (!q || hay(p).includes(q)));
}
