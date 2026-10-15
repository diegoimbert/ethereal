// #563: shown instead of the editor while another tab owns the project store (`storeLock.ts`).
import { Copy } from "lucide-react";
import { useState } from "react";
import { Button } from "@/kit";
import { useT } from "@/i18n";
import { StartupScreen } from "./StartupScreen";

export interface OtherTabScreenProps {
  /** `busy`: this tab never started; `moved`: it handed its store over to another tab. */
  reason: "busy" | "moved";
  /** "use here": take the store over (resolves once this tab has it). */
  onUseHere(): Promise<void>;
}

export function OtherTabScreen({ reason, onUseHere }: OtherTabScreenProps) {
  const t = useT();
  const [waiting, setWaiting] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const useHere = () => {
    setWaiting(true);
    setFailed(null);
    onUseHere().catch((e: unknown) => {
      setWaiting(false);
      setFailed(e instanceof Error ? e.message : String(e));
    });
  };
  return (
    <StartupScreen
      testId="other-tab"
      icon={<Copy />}
      title={reason === "busy" ? t("startup.other_tab.busy") : t("startup.other_tab.moved")}
      actions={
        <Button tone="accent" size="lg" disabled={waiting} onClick={useHere}>
          {waiting ? t("startup.other_tab.waiting") : t("startup.other_tab.use_here")}
        </Button>
      }
    >
      <p className="eth-startup__text">{reason === "busy" ? t("startup.other_tab.busy_hint") : t("startup.other_tab.moved_hint")}</p>
      {failed && (
        <p className="eth-startup__text" role="alert" data-i18n-exempt>
          {failed}
        </p>
      )}
    </StartupScreen>
  );
}
