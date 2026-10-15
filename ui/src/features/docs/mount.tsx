// The docs page (`/docs`, `/docs/<section>/<page>`), loaded by the web entry instead of the
// app: React, the design tokens and the docs, nothing else (no editor, no engine, no wasm, no
// project store). Works on phones.
import "@fontsource-variable/inter";
import "@/kit/theme.css";
import "@/kit/kit.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { DocsApp } from "./DocsApp";
import { useUrlRouter } from "./router";
import { initI18n, t } from "@/i18n";
import { setTabTitle, tabTitle } from "@/features/document-title/title";

/** The app's remembered theme (Settings > Appearance), else the system's. */
function applyTheme(): void {
  let theme: string | null = null;
  try {
    theme = localStorage.getItem("eth-theme");
  } catch {
    // storage unavailable
  }
  if (theme !== "light" && theme !== "dark") theme = matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.background = "";
}

function StandaloneDocs() {
  const router = useUrlRouter();
  return <DocsApp router={router} />;
}

export async function mountDocsPage(root: HTMLElement): Promise<void> {
  applyTheme();
  // The app's language (Settings > general), else the system's.
  await initI18n();
  setTabTitle(tabTitle(t("docs.tab_title")));
  createRoot(root).render(
    <StrictMode>
      <StandaloneDocs />
    </StrictMode>,
  );
}
