import clsx from "clsx";
import type { HTMLAttributes, ReactNode } from "react";

export interface PanelProps extends Omit<HTMLAttributes<HTMLElement>, "title"> {
  /** Header label. Omit for a headerless panel. */
  title?: ReactNode;
  /** Right-aligned header content (buttons, toggles). */
  actions?: ReactNode;
  bodyClassName?: string;
}

export function Panel({ title, actions, className, bodyClassName, children, ...rest }: PanelProps) {
  return (
    <section className={clsx("eth-panel", className)} {...rest}>
      {(title !== undefined || actions !== undefined) && (
        <header className="eth-panel__header">
          {title}
          {actions !== undefined && <div className="eth-panel__actions">{actions}</div>}
        </header>
      )}
      <div className={clsx("eth-panel__body", bodyClassName)}>{children}</div>
    </section>
  );
}
