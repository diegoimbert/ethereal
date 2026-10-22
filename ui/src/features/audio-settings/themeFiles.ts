// eth-allow-case-file: file names, mime types and IPC command names.
/**
 * Theme files (Settings > Appearance > export… / import theme…): the browser downloads and
 * picks them; the desktop app saves and opens them through its native dialogs
 * (`text_file_save` / `text_file_open`, apps/desktop/src-tauri/src/text_file.rs), as the
 * webview can do neither.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { saveFile } from "@/features/export/download";
import { serializeTheme, type ThemeDefinition } from "@/theme";

const FILTER = "ethereal theme";
const EXTENSIONS = ["json"];

/** Injectable for tests. */
export const themeFileIpc = {
  invoke: tauriInvoke as <T>(cmd: string, args: Record<string, unknown>) => Promise<T>,
  desktop: (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window,
};

/** `my theme` → `my-theme.json`. */
export function themeFileName(name: string): string {
  const slug = name
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return `${slug || "theme"}.json`;
}

/**
 * Saves `def` as a theme file. Resolves where it went on the desktop (`null`: dismissed), or
 * `""` for a browser download.
 */
export async function exportThemeFile(def: Pick<ThemeDefinition, "name" | "base" | "colors">): Promise<string | null> {
  const text = serializeTheme(def);
  const name = themeFileName(def.name);
  if (themeFileIpc.desktop()) {
    return themeFileIpc.invoke<string | null>("text_file_save", { name, contents: text, filter: FILTER, extensions: EXTENSIONS });
  }
  saveFile(name, "application/json", new TextEncoder().encode(text));
  return "";
}

/** The browser's file picker (one `.json`); `null` when dismissed. */
function pickInBrowser(): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json,application/json";
    input.hidden = true;
    input.setAttribute("data-testid", "theme-file-input");
    let done = false;
    const finish = (f: File | null) => {
      if (done) return;
      done = true;
      input.remove();
      resolve(f);
    };
    input.addEventListener("change", () => finish(input.files?.[0] ?? null));
    input.addEventListener("cancel", () => finish(null));
    document.body.appendChild(input);
    input.click();
  });
}

/** Asks for a theme file and reads it: its text, or `null` when dismissed. */
export async function openThemeFile(): Promise<string | null> {
  if (themeFileIpc.desktop()) return themeFileIpc.invoke<string | null>("text_file_open", { filter: FILTER, extensions: EXTENSIONS });
  const file = await pickInBrowser();
  return file ? file.text() : null;
}
