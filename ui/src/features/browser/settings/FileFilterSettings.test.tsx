import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { useContext } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { FileFilter } from "@/generated";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { cmd, TransportContext } from "@/transport";
import type { MockTransport } from "@/transport/mock/MockTransport";
import { defaultFileFilter } from "@/transport/mock/roadmap/fileFilter";
import { useConfirm } from "@/kit";
import { FileFilterSettings, resetFileFilterCache } from "./FileFilterSettings";

afterEach(() => {
  resetStores();
  resetFileFilterCache();
  localStorage.removeItem("eth.library.regexRules");
});

async function stored(mock: MockTransport): Promise<FileFilter> {
  const r = await mock.send(cmd("Browser", { type: "GetFileFilter" }));
  if (r.type !== "BrowserFileFilter") throw new Error(r.type);
  return r.filter;
}

async function setup() {
  const r = await renderWithMock(<Host />);
  await screen.findByRole("group", { name: "show rules" });
  return r;
}

/** The panel takes the transport as a prop (as the settings dialog passes it). */
function Host() {
  const transport = useContext(TransportContext)?.transport;
  return transport ? <FileFilterSettings transport={transport} /> : null;
}

const show = () => within(screen.getByRole("group", { name: "show rules" }));
const hide = () => within(screen.getByRole("group", { name: "hide rules" }));
const count = () => screen.getByTestId("file-filter-count");

describe("FileFilterSettings (regex mode)", () => {
  beforeEach(() => localStorage.setItem("eth.library.regexRules", "1"));

  it("shows the default rules and how many library files they show", async () => {
    await setup();
    const defaults = defaultFileFilter();
    expect(
      show()
        .getAllByRole("textbox")
        .map((i) => (i as HTMLInputElement).value),
    ).toEqual(defaults.whitelist);
    expect(
      hide()
        .getAllByRole("textbox")
        .map((i) => (i as HTMLInputElement).value),
    ).toEqual(defaults.blacklist);
    expect(screen.getByRole("button", { name: "reset to defaults" })).toBeDisabled();
    await waitFor(() => expect(count()).toHaveTextContent("shows 10 of 13 files in your library (hides 3)."));
  });

  it("adds a rule (saved on blur) and resets to the defaults", async () => {
    const { mock } = await setup();
    fireEvent.click(screen.getByRole("button", { name: "add show rule" }));
    const input = show().getByRole("textbox", { name: "show rule 4" });
    expect(input).toHaveFocus();
    fireEvent.change(input, { target: { value: String.raw`\.(txt|als)$` } });
    // Live preview before saving.
    await waitFor(() => expect(count()).toHaveTextContent("shows 12 of 13"));
    expect((await stored(mock)).whitelist).toHaveLength(3);
    fireEvent.blur(input);
    await waitFor(async () => expect((await stored(mock)).whitelist).toEqual([...defaultFileFilter().whitelist, String.raw`\.(txt|als)$`]));

    const reset = screen.getByRole("button", { name: "reset to defaults" });
    await waitFor(() => expect(reset).toBeEnabled());
    fireEvent.click(reset);
    // It asks first.
    expect(await stored(mock)).not.toEqual(defaultFileFilter());
    const ask = useConfirm.getState().request!;
    expect(ask.title).toBe("reset the library file rules?");
    act(() => ask.run());
    await waitFor(() => expect(show().getAllByRole("textbox")).toHaveLength(3));
    expect(await stored(mock)).toEqual(defaultFileFilter());
  });

  it("marks a rule that isn't a regex and doesn't save it", async () => {
    const { mock } = await setup();
    const input = hide().getByRole("textbox", { name: "hide rule 2" });
    fireEvent.change(input, { target: { value: "Thumbs(" } });
    await waitFor(() => expect(input).toHaveAttribute("aria-invalid", "true"));
    const describedBy = input.getAttribute("aria-describedby");
    expect(describedBy && document.getElementById(describedBy)).toHaveTextContent(/./);
    expect(count()).toHaveTextContent("fix the marked rules");
    fireEvent.keyDown(input, { key: "Enter" });
    fireEvent.blur(input);
    await new Promise((r) => setTimeout(r, 50));
    expect(await stored(mock)).toEqual(defaultFileFilter());
    // Fixed: the error goes away and the rule saves.
    fireEvent.change(input, { target: { value: String.raw`^Thumbs\.db$|^ehthumbs\.db$` } });
    await waitFor(() => expect(input).not.toHaveAttribute("aria-invalid"));
    fireEvent.blur(input);
    await waitFor(async () => expect((await stored(mock)).blacklist[1]).toBe(String.raw`^Thumbs\.db$|^ehthumbs\.db$`));
  });

  it("removes a rule and drops blank ones", async () => {
    const { mock } = await setup();
    fireEvent.click(hide().getByRole("button", { name: "remove hide rule 1" }));
    await waitFor(async () => expect((await stored(mock)).blacklist).toEqual(defaultFileFilter().blacklist.slice(1)));
    fireEvent.click(screen.getByRole("button", { name: "add hide rule" }));
    const blank = hide().getByRole("textbox", { name: "hide rule 5" });
    fireEvent.blur(blank);
    await waitFor(() => expect(hide().queryByRole("textbox", { name: "hide rule 5" })).toBeNull());
    expect(await stored(mock)).toEqual({ ...defaultFileFilter(), blacklist: defaultFileFilter().blacklist.slice(1) });
  });

  it("follows a change made elsewhere", async () => {
    const { mock } = await setup();
    await mock.send(cmd("Browser", { type: "SetFileFilter", filter: { whitelist: [], blacklist: ["x"] } }));
    await waitFor(() => expect(show().queryAllByRole("textbox")).toHaveLength(0));
    expect(hide().getByRole("textbox", { name: "hide rule 1" })).toHaveValue("x");
  });
});

describe("FileFilterSettings (plain terms, the default)", () => {
  const kinds = (g: ReturnType<typeof show>) => g.getAllByRole("combobox").map((c) => c.textContent);
  const values = (g: ReturnType<typeof show>) => g.getAllByRole("textbox").map((i) => (i as HTMLInputElement).value);

  it("shows each rule as a kind and its text; a custom pattern as its regex", async () => {
    const { mock } = await setup();
    expect(kinds(show())[0]).toBe("file type");
    expect(values(show())[0]).toMatch(/^wav, wave, aif/);
    const hidden = defaultFileFilter().blacklist.map((rule, i) => [rule, kinds(hide())[i], values(hide())[i]]);
    expect(hidden).toContainEqual([String.raw`^\.`, "name starts with", "."]);
    expect(hidden).toContainEqual([String.raw`^Thumbs\.db$`, "name is", "Thumbs.db"]);
    await act(async () => {
      await mock.send(cmd("Browser", { type: "SetFileFilter", filter: { whitelist: ["^take[0-9]+"], blacklist: [] } }));
    });
    await waitFor(() => expect(kinds(show())).toEqual(["pattern (regex)"]));
    expect(values(show())).toEqual(["^take[0-9]+"]);
  });

  it("a new rule stays when its kind menu is pressed (WebKit: the press doesn't move the focus)", async () => {
    await setup();
    fireEvent.click(screen.getByRole("button", { name: "add show rule" }));
    const input = show().getByRole("textbox", { name: "show rule 4" });
    const kind = show().getByRole("combobox", { name: "kind of show rule 4" });
    fireEvent.pointerDown(kind);
    fireEvent.blur(input, { relatedTarget: null });
    expect(show().getByRole("textbox", { name: "show rule 4" })).toBeInTheDocument();
    // Leaving it blank by other means still drops it.
    await new Promise((r) => setTimeout(r, 0));
    fireEvent.blur(show().getByRole("textbox", { name: "show rule 4" }), { relatedTarget: null });
    await waitFor(() => expect(show().queryByRole("textbox", { name: "show rule 4" })).toBeNull());
  });

  it("adds a file type in plain words: saved as the rule it means", async () => {
    const { mock } = await setup();
    fireEvent.click(screen.getByRole("button", { name: "add show rule" }));
    const input = show().getByRole("textbox", { name: "show rule 4" });
    fireEvent.change(input, { target: { value: "txt, als" } });
    fireEvent.blur(input);
    await waitFor(async () => expect((await stored(mock)).whitelist).toEqual([...defaultFileFilter().whitelist, String.raw`\.(txt|als)$`]));
  });
});
