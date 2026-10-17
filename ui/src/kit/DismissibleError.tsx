import type { ReactNode } from "react";
import { useT } from "@/i18n";

export interface DismissibleErrorProps {
  /** The button's class (the feature's error style). */
  className: string;
  onDismiss: () => void;
  children: ReactNode;
}

/**
 * An error message that is dismissed by pressing it. The alert role sits on a wrapper (a
 * button given it would lose its own role); `display: contents` keeps the button where the
 * feature's layout puts it.
 */
export function DismissibleError({ className, onDismiss, children }: DismissibleErrorProps) {
  const t = useT();
  return (
    <div role="alert" className="eth-alert">
      <button type="button" className={className} title={t("common.dismiss")} onClick={onDismiss}>
        {children}
      </button>
    </div>
  );
}
