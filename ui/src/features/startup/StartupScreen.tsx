// A full-page notice shown instead of the editor (apps/web renders it before or in place of
// the app): an icon, a title, a few lines and the actions.
import "./startup.css";
import type { ReactNode } from "react";

export interface StartupScreenProps {
  icon: ReactNode;
  title: string;
  children?: ReactNode;
  actions?: ReactNode;
  testId?: string;
}

export function StartupScreen({ icon, title, children, actions, testId }: StartupScreenProps) {
  return (
    <main className="eth-startup" data-testid={testId}>
      <section className="eth-startup__card" aria-labelledby="eth-startup-title">
        <span className="eth-startup__icon" aria-hidden>
          {icon}
        </span>
        <h1 id="eth-startup-title" className="eth-startup__title">
          {title}
        </h1>
        {children}
        {actions && <div className="eth-startup__actions">{actions}</div>}
      </section>
    </main>
  );
}
