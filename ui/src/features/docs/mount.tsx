// The docs page (`/docs`, `/docs/<section>/<page>`), loaded by the web entry instead of the
// app: React, the design tokens and the docs, nothing else (no editor, no engine, no wasm, no
// project store). Works on phones.
import "@fontsource-variable/inter";
import "@/kit/theme.css";
import "@/kit/kit.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { DocsApp } from "./DocsApp";
import { loadDocsExtensions } from "./extensions";
import { useUrlRouter } from "./router";
import { initI18n, t } from "@/i18n";
import { setTabTitle, tabTitle } from "@/features/document-title/title";
import { initThemeSelection } from "@/theme";

/** The app's remembered theme (Settings > Appearance; following the system there by default). */
function applyTheme(): void {
  let chosen = false;
  try {
    chosen = localStorage.getItem("eth-theme") !== null || localStorage.getItem("eth-theme-mode") !== null;
  } catch {
    // storage unavailable
  }
  // Never chosen in the app: the system's light or dark.
  if (!chosen && matchMedia("(prefers-color-scheme: light)").matches) document.documentElement.dataset.theme = "light";
  else initThemeSelection();
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
  // The pages a build adds lazily (`extendDocsLazily`).
  await loadDocsExtensions();
  setTabTitle(tabTitle(t("docs.tab_title")));
  createRoot(root).render(
    <StrictMode>
      <StandaloneDocs />
    </StrictMode>,
  );
}
