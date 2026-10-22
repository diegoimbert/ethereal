import { afterEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useConfirm, useContextMenuStore, type ContextMenuItem } from "@/kit";
import { deleteCustomTheme, getThemes, previewTheme, serializeTheme, setThemeId, themeColorValue, THEME_FORMAT } from "@/theme";
import { AppearanceSettings } from "./AppearanceSettings";
import { themeFileIpc } from "./themeFiles";

const root = document.documentElement;
const ipc = { ...themeFileIpc };

afterEach(() => {
  Object.assign(themeFileIpc, ipc);
  previewTheme(null);
  for (const d of getThemes()) if (d.id.startsWith("custom-")) deleteCustomTheme(d.id);
  setThemeId("dark");
  useContextMenuStore.setState({ menu: null });
  useConfirm.setState({ request: null });
});

/** Desktop file dialogs, answered by `files` (`text_file_open` → its text; `text_file_save` → a path). */
function desktop(open: string | null = null) {
  const invoke = vi.fn(async (cmd: string) => (cmd === "text_file_open" ? open : "/home/me/themes/x.json"));
  Object.assign(themeFileIpc, { desktop: () => true, invoke });
  return invoke;
}

const grid = () => within(screen.getByRole("group", { name: "theme" }));

/** Picks `item` in the menu of the card named `card`. */
function cardMenu(card: string, item: string) {
  fireEvent.click(screen.getByRole("button", { name: `${card}: more` }));
  const entry = useContextMenuStore.getState().menu!.items.find((i): i is ContextMenuItem => i !== "separator" && i.label === item);
  expect(entry, item).toBeDefined();
  act(() => entry!.onSelect());
}

describe("theme editor", () => {
  it("creates a theme from the current one, previews it live on the whole app, and saves it", async () => {
    setThemeId("obsidian");
    render(<AppearanceSettings />);
    fireEvent.click(screen.getByRole("button", { name: "create your own theme" }));
    const editor = screen.getByTestId("theme-editor");
    expect(within(editor).getByRole("textbox", { name: "name" })).toHaveValue("obsidian copy");
    // The main colors start from obsidian's.
    expect(within(editor).getByRole("textbox", { name: "accent value" })).toHaveValue("#3fc8e4");

    fireEvent.change(within(editor).getByRole("textbox", { name: "accent value" }), { target: { value: "#ff8844" } });
    // The whole app shows the draft (the preview, once a frame), not chosen yet.
    await waitFor(() => expect(themeColorValue("accent")).toBe("#ff8844"));
    expect(root.dataset.themeId).toBe("preview");
    expect(localStorage.getItem("eth-theme-id")).toBe("obsidian");
    // Derived tokens follow.
    expect(themeColorValue("selection")).toBe("#ff8844");

    fireEvent.change(within(editor).getByRole("textbox", { name: "name" }), { target: { value: "sunset" } });
    fireEvent.click(within(editor).getByRole("button", { name: "save" }));
    expect(screen.queryByTestId("theme-editor")).toBeNull();
    const saved = getThemes().find((d) => d.name === "sunset")!;
    expect(saved.base).toBe("dark");
    expect(saved.colors.accent).toBe("#ff8844");
    expect(root.dataset.themeId).toBe(saved.id);
    expect(grid().getByRole("button", { name: "sunset" })).toHaveAttribute("aria-pressed", "true");
    expect(JSON.parse(localStorage.getItem("eth-custom-themes")!)).toEqual([saved]);
  });

  it("lists the contrast checks a draft fails, and cancel drops the preview", async () => {
    render(<AppearanceSettings />);
    fireEvent.click(screen.getByRole("button", { name: "create your own theme" }));
    const editor = screen.getByTestId("theme-editor");
    const contrast = within(editor).getByTestId("theme-contrast");
    expect(contrast).toHaveTextContent("text and controls pass the wcag aa contrast checks.");
    fireEvent.change(within(editor).getByRole("textbox", { name: "dim text value" }), { target: { value: "#333333" } });
    expect(contrast).toHaveTextContent(/text-dim on bg: contrast 1\.\d, needs 4\.5\./);
    // A derived token can be set, then reset to the derived value.
    fireEvent.click(within(editor).getByText("all tokens"));
    const row = within(editor).getByTestId("theme-color-selection");
    fireEvent.change(within(row).getByRole("textbox"), { target: { value: "#00ff00" } });
    expect(row).toHaveAttribute("data-set", "true");
    fireEvent.click(within(row).getByRole("button", { name: "reset selection" }));
    expect(row).not.toHaveAttribute("data-set");
    expect(within(row).getByRole("textbox")).toHaveValue("#d48fd0");

    await waitFor(() => expect(root.dataset.themeId).toBe("preview"));
    fireEvent.click(within(editor).getByRole("button", { name: "cancel" }));
    expect(root.dataset.themeId).toBe("dark");
    expect(getThemes().some((d) => d.id.startsWith("custom-"))).toBe(false);
  });

  it("edits, duplicates and deletes a theme of the user's from its card's menu", () => {
    render(<AppearanceSettings />);
    fireEvent.click(screen.getByRole("button", { name: "create your own theme" }));
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    expect(grid().getByRole("button", { name: "dark copy" })).toBeInTheDocument();

    cardMenu("dark copy", "edit");
    fireEvent.change(screen.getByRole("textbox", { name: "name" }), { target: { value: "night" } });
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    expect(grid().getByRole("button", { name: "night" })).toBeInTheDocument();
    expect(getThemes().filter((d) => d.id.startsWith("custom-"))).toHaveLength(1);

    cardMenu("night", "duplicate");
    expect(screen.getByRole("textbox", { name: "name" })).toHaveValue("night copy");
    fireEvent.click(screen.getByRole("button", { name: "cancel" }));

    cardMenu("night", "delete");
    act(() => useConfirm.getState().request!.run());
    expect(grid().queryByRole("button", { name: "night" })).toBeNull();
    expect(root.dataset.themeId).toBe("dark");
  });
});

describe("theme files", () => {
  it("exports a theme as a theme file (desktop: the save dialog)", async () => {
    const invoke = desktop();
    render(<AppearanceSettings />);
    cardMenu("tide", "export…");
    await screen.findByText("saved to /home/me/themes/x.json.");
    expect(invoke).toHaveBeenCalledWith("text_file_save", {
      name: "tide.json",
      contents: expect.stringContaining(`"format": "${THEME_FORMAT}"`),
      filter: "ethereal theme",
      extensions: ["json"],
    });
    const file = JSON.parse((invoke.mock.calls[0] as unknown as [string, { contents: string }])[1].contents);
    expect(file).toMatchObject({ version: 1, name: "tide", base: "dark", colors: { accent: "#8aa4ff" } });
  });

  it("imports a theme file, applies it and says what it repaired", async () => {
    const text = JSON.stringify({ format: THEME_FORMAT, version: 1, name: "moss", base: "dark", colors: { accent: "#7fbf6a", glitter: "#fff" } });
    desktop(text);
    render(<AppearanceSettings />);
    fireEvent.click(screen.getByRole("button", { name: "import theme…" }));
    expect(await screen.findByRole("status")).toHaveTextContent("imported moss. ignored tokens ethereal doesn't know: glitter.");
    expect(grid().getByRole("button", { name: "moss" })).toHaveAttribute("aria-pressed", "true");
    expect(themeColorValue("accent")).toBe("#7fbf6a");
  });

  it("refuses a file from a newer version, or that isn't a theme", async () => {
    desktop(serializeTheme({ name: "x", base: "dark", colors: {} }).replace('"version": 1', '"version": 7'));
    render(<AppearanceSettings />);
    fireEvent.click(screen.getByRole("button", { name: "import theme…" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("this theme was made by a newer version of ethereal (format 7). update ethereal to import it.");
    desktop("not json");
    fireEvent.click(screen.getByRole("button", { name: "import theme…" }));
    await screen.findByText("this file isn't valid json.");
    expect(getThemes().some((d) => d.id.startsWith("custom-"))).toBe(false);
  });
});
