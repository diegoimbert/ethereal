import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { AU_DELAY, DesktopFake, EQ, plugin, SYNTH, VERB, VST2_VERB, VST_VERB } from "@/features/plugins/testing";
import { platformFormats } from "@/features/plugins/filter";
import { usePluginVisibility } from "@/features/plugins/visibility";
import { useProjectStore, useSelectionStore } from "@/state";
import { TransportProvider, type EngineTransport } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { libraryKey, recordDeviceUse, resetLibraryStats, useLibraryStats } from "./libraryStats";
import { deviceSpec } from "./libraryInsert";
import type { LibraryDevicePayload } from "./libraryDevice";
import { pickOption } from "@/kit/testing";
import { useAudioSettings } from "@/features/audio-settings/store";
import { DevicePicker } from "./DevicePicker";
import { DeviceLibrary } from "./DeviceLibrary";
import { useContextMenuStore } from "@/kit/contextMenuStore";
import { useNotices } from "@/features/notifications";
import { resetI18nForTests, setLocale } from "@/i18n/runtime";

let transport: EngineTransport | undefined;
afterEach(() => {
  cleanup();
  transport?.dispose();
  transport = undefined;
  useProjectStore.getState().reset();
  useSelectionStore.getState().selectTrack(null);
  usePluginStore.getState().reset();
  resetLibraryStats();
  localStorage.removeItem("eth.plugins.visibility");
  usePluginVisibility.getState().reload();
  resetI18nForTests();
});

async function renderWith<T extends EngineTransport>(t: T, ui: ReactNode): Promise<T> {
  transport = t;
  render(<TransportProvider transport={t}>{ui}</TransportProvider>);
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  return t;
}

const rows = async (name: string) => {
  const list = await screen.findByRole("list", { name });
  await waitFor(() => expect(list).not.toHaveAttribute("aria-busy", "true"));
  return within(list);
};
const names = (list: Awaited<ReturnType<typeof rows>>) => list.queryAllByRole("listitem").map((li) => li.textContent);

describe("DeviceLibrary", () => {
  it("lists factory devices and plugins (desktop) of its category by name, with All / Factory / Plugins and search", async () => {
    const t = new DesktopFake();
    t.plugins = [VERB, SYNTH, EQ, AU_DELAY];
    await renderWith(t, <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    await list.findByText("Air EQ");
    const all = names(list);
    expect(all).toContain("compressor");
    expect(all).not.toContain("synth");
    // All by name (factory and plugins mixed), plugins with their vendor and format chip.
    expect(all).toContain("VerbAcmeclap");
    expect(all).toEqual([...all].sort((a, b) => a!.localeCompare(b!)));

    fireEvent.click(screen.getByRole("button", { name: "plugins" }));
    expect(names(list)).toEqual(["Air EQOtherclap", "AUDelayAppleau", "VerbAcmeclap"]);
    fireEvent.click(screen.getByRole("button", { name: "factory" }));
    expect(names(list).some((n) => n?.includes("Acme"))).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "all" }));
    fireEvent.change(screen.getByRole("searchbox", { name: "search effects" }), { target: { value: "apple" } });
    expect(names(list)).toEqual(["AUDelayAppleau"]);
    fireEvent.change(screen.getByRole("searchbox", { name: "search effects" }), { target: { value: "zzz" } });
    expect(list.getByText("no match.")).toBeInTheDocument();
  });

  it('names the EQ "eq" in lowercase, like every factory effect, and search finds it either way', async () => {
    await renderWith(new MockTransport({ timers: "manual", seed: 3 }), <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    await list.findByText("eq");
    for (const n of names(list)) expect(n).toBe(n!.toLowerCase());
    const search = screen.getByRole("searchbox", { name: "search effects" });
    for (const q of ["EQ", "eq", "Eq"]) {
      fireEvent.change(search, { target: { value: q } });
      expect(names(list)).toContain("eq");
    }
    await act(() => setLocale("fr"));
    fireEvent.change(search, { target: { value: "EQ" } });
    expect(names(list)).toContain("eq");
  });

  it("names acronyms lowercase too (midi effect rack, fm synth), and search finds them either way", async () => {
    const cases = [
      ["NoteEffect", "midi effects", "midi effect rack", ["MIDI", "midi", "Midi"]],
      ["Instrument", "instruments", "fm synth", ["FM", "fm"]],
    ] as const;
    for (const [category, label, name, queries] of cases) {
      await renderWith(new MockTransport({ timers: "manual", seed: 3 }), <DeviceLibrary category={category} onPick={() => undefined} />);
      const list = await rows(label);
      await list.findByText(name);
      for (const n of names(list)) expect(n).toBe(n!.toLowerCase());
      const search = screen.getByRole("searchbox", { name: `search ${label}` });
      for (const q of queries) {
        fireEvent.change(search, { target: { value: q } });
        expect(names(list)).toContain(name);
      }
      cleanup();
    }
  });

  it("hides built-in types the engine flags unimplemented (contract placeholders, #438)", async () => {
    const t = new MockTransport({ timers: "manual", seed: 3 });
    const send = t.send.bind(t);
    // The engine still lists them (flagged), so a project holding one names it.
    t.send = async (command, opts) => {
      const reply = await send(command, opts);
      if (reply.type !== "DeviceTypes") return reply;
      return { ...reply, devices: reply.devices.map((d) => (d.name === "Delay" ? { ...d, implemented: false } : d)) };
    };
    await renderWith(t, <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    await list.findByText("reverb");
    expect(names(list)).not.toContain("Delay");
    fireEvent.change(screen.getByRole("searchbox", { name: "search effects" }), { target: { value: "delay" } });
    expect(names(list).some((n) => n?.startsWith("Delay"))).toBe(false);
  });

  it("favourites (row stars, favourites-only filter) and sorts (name, most used, most recent), remembered", async () => {
    await renderWith(new MockTransport({ timers: "manual", seed: 3 }), <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    await list.findByText("delay");
    fireEvent.click(list.getByRole("button", { name: "favourite delay" }));
    expect(list.getByRole("button", { name: "unfavourite delay" })).toHaveClass("eth-devlib__star--on");
    fireEvent.click(screen.getByRole("button", { name: "favourites only" }));
    expect(names(list)).toEqual(["delay"]);
    fireEvent.click(screen.getByRole("button", { name: "favourites only" }));

    // Use Reverb twice and Limiter once (any add counts), Limiter last.
    const use = (name: string) => act(() => recordDeviceUse({ device: { type: "Builtin", builtin: name as never } }));
    use("Reverb");
    use("Reverb");
    use("Limiter");
    pickOption(screen.getByRole("combobox", { name: "sort" }), { value: "MostUsed" });
    expect(names(list).slice(0, 2)).toEqual(["reverb", "limiter"]);
    pickOption(screen.getByRole("combobox", { name: "sort" }), { value: "Recent" });
    expect(names(list)[0]).toBe("limiter");
    expect(JSON.parse(localStorage.getItem("eth-device-library")!)).toMatchObject({ sort: "Recent", favourites: ["builtin:Delay"] });
  });

  it("picks with a click (stars don't pick)", async () => {
    const picked: string[] = [];
    await renderWith(new DesktopFake(), <DeviceLibrary category="Instrument" onPick={(p) => picked.push(p.name)} />);
    const list = await rows("instruments");
    fireEvent.click((await list.findByText("Big Synth")).closest<HTMLElement>("[data-key]")!);
    fireEvent.click(list.getByRole("button", { name: "favourite Big Synth" }));
    expect(picked).toEqual(["Big Synth"]);
  });

  it("walks the rows from the search field; Enter picks the active one", async () => {
    const picked: string[] = [];
    await renderWith(new DesktopFake(), <DeviceLibrary category="Instrument" onPick={(p) => picked.push(p.name)} />);
    const list = await rows("instruments");
    await list.findByText("Big Synth");
    const field = screen.getByRole("searchbox", { name: "search instruments" });
    fireEvent.change(field, { target: { value: "big" } });
    fireEvent.keyDown(field, { key: "ArrowDown" });
    expect(list.getByText("Big Synth").closest("[data-key]")).toHaveClass("eth-devlib__row--active");
    fireEvent.keyDown(field, { key: "Enter" });
    expect(picked).toEqual(["Big Synth"]);
  });

  it("the arrows wrap: Up on the first row goes to the last, Down on the last back to the first (scrolled into view)", async () => {
    const scrolled: (string | undefined)[] = [];
    // jsdom has no scrollIntoView: record which row the library scrolls to.
    HTMLElement.prototype.scrollIntoView = function (this: HTMLElement) {
      scrolled.push(this.dataset.key);
    };
    try {
      await renderWith(new MockTransport({ timers: "manual", seed: 3 }), <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
      const list = await rows("effects");
      await list.findByText("delay");
      const field = screen.getByRole("searchbox", { name: "search effects" });
      const keys = () => Array.from(document.querySelectorAll<HTMLElement>(".eth-devlib__row")).map((r) => r.dataset.key);
      const active = () => document.querySelector<HTMLElement>(".eth-devlib__row--active")?.dataset.key;
      const all = keys();
      expect(all.length).toBeGreaterThan(2);
      expect(active()).toBe(all[0]);
      fireEvent.keyDown(field, { key: "ArrowUp" });
      expect(active()).toBe(all.at(-1));
      expect(scrolled.at(-1)).toBe(all.at(-1));
      fireEvent.keyDown(field, { key: "ArrowDown" });
      expect(active()).toBe(all[0]);
      expect(scrolled.at(-1)).toBe(all[0]);

      // A search narrows the rows: the wrap follows the filtered list.
      fireEvent.change(field, { target: { value: "e" } });
      const shown = keys();
      expect(shown.length).toBeGreaterThan(1);
      expect(shown.length).toBeLessThanOrEqual(all.length);
      fireEvent.keyDown(field, { key: "ArrowUp" });
      expect(active()).toBe(shown.at(-1));
      fireEvent.keyDown(field, { key: "ArrowDown" });
      expect(active()).toBe(shown[0]);

      // A single row stays put both ways.
      fireEvent.change(field, { target: { value: "limiter" } });
      const one = keys();
      expect(one).toHaveLength(1);
      fireEvent.keyDown(field, { key: "ArrowDown" });
      expect(active()).toBe(one[0]);
      fireEvent.keyDown(field, { key: "ArrowUp" });
      expect(active()).toBe(one[0]);
    } finally {
      delete (HTMLElement.prototype as Partial<HTMLElement>).scrollIntoView;
    }
  });

  it("the first row is active right after typing: Enter picks it without an arrow", async () => {
    const picked: string[] = [];
    await renderWith(new DesktopFake(), <DeviceLibrary category="Instrument" onPick={(p) => picked.push(p.name)} />);
    const list = await rows("instruments");
    await list.findByText("Big Synth");
    const field = screen.getByRole("searchbox", { name: "search instruments" });
    fireEvent.change(field, { target: { value: "big" } });
    expect(list.getByText("Big Synth").closest("[data-key]")).toHaveClass("eth-devlib__row--active");
    fireEvent.keyDown(field, { key: "Enter" });
    expect(picked).toEqual(["Big Synth"]);
  });

  it("shows scan progress and failures in the footer, with a Plugin settings button (no Rescan)", async () => {
    const t = await renderWith(new DesktopFake(), <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    await list.findByText("Verb");
    expect(screen.queryByRole("button", { name: "rescan" })).toBeNull();
    expect(screen.getByRole("button", { name: "plugin settings" })).toBeInTheDocument();
    t.emit({ type: "ScanProgress", done: 1, total: 3, current: "/plugins/Air EQ.clap" });
    expect(screen.getByText("scanning 1/3")).toBeInTheDocument();
    t.plugins = [VERB, SYNTH, EQ];
    t.emit({ type: "ScanFinished", plugins: 3, failed: [{ path: "/plugins/bad.clap", message: "crashed" }] });
    await list.findByText("Air EQ");
    expect(screen.getByText("1 plugin failed to load")).toBeInTheDocument();
  });

  it("explains plugins are desktop-only on the web", async () => {
    await renderWith(new MockTransport({ timers: "manual", seed: 3 }), <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    await rows("effects");
    fireEvent.click(screen.getByRole("button", { name: "plugins" }));
    expect(screen.getByText("plugins are available in the desktop app.")).toBeInTheDocument();
  });

  it("lists several categories at once (a track's add menu), narrowed by `accept` (what fits a rack)", async () => {
    const t = new DesktopFake();
    t.plugins = [VERB, SYNTH, EQ];
    const picked: string[] = [];
    await renderWith(t, <DeviceLibrary categories={["Instrument", "AudioEffect"]} accept={(p) => p.name !== "Verb"} onPick={(p) => picked.push(p.name)} />);
    const list = await rows("devices");
    await list.findByText("Air EQ");
    const all = names(list);
    expect(all).toContain("Big SynthAcmeclap");
    expect(all).toContain("compressor");
    expect(all).toContain("synth");
    expect(all).not.toContain("VerbAcmeclap");
    expect(screen.getByRole("searchbox", { name: "search devices" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "plugin settings" })).toBeInTheDocument();
    fireEvent.click(list.getByRole("button", { name: "Air EQ" }));
    expect(picked).toEqual(["Air EQ"]);
  });

  it("plugins only, from a given list (Replace plugin, Resolve): no factory devices or source chips, same search and footer", async () => {
    const picked: string[] = [];
    let closed = 0;
    await renderWith(
      new DesktopFake(),
      <DeviceLibrary pluginsOnly plugins={[VERB, EQ, AU_DELAY]} onClose={() => closed++} onPick={(p) => picked.push(p.name)} />,
    );
    const list = await rows("plugins");
    expect(names(list)).toEqual(["Air EQOtherclap", "AUDelayAppleau", "VerbAcmeclap"]);
    expect(screen.queryByRole("button", { name: "factory" })).toBeNull();
    fireEvent.change(screen.getByRole("searchbox", { name: "search plugins" }), { target: { value: "apple" } });
    expect(names(list)).toEqual(["AUDelayAppleau"]);
    fireEvent.keyDown(screen.getByRole("searchbox", { name: "search plugins" }), { key: "ArrowDown" });
    fireEvent.keyDown(screen.getByRole("searchbox", { name: "search plugins" }), { key: "Enter" });
    expect(picked).toEqual(["AUDelay"]);
    fireEvent.click(screen.getByRole("button", { name: "plugin settings" }));
    expect(closed).toBe(1);
    expect(useAudioSettings.getState()).toMatchObject({ open: true, tab: "plugins" });
    useAudioSettings.getState().close();
  });

  it("DevicePicker: a trigger opening the library in a popover; picking closes it", async () => {
    const picked: string[] = [];
    await renderWith(
      new DesktopFake(),
      <DevicePicker aria-label="add effect" category="AudioEffect" onPick={(p) => picked.push(p.name)}>
        + Add effect
      </DevicePicker>,
    );
    fireEvent.click(screen.getByRole("button", { name: "add effect" }));
    const popover = await screen.findByRole("dialog", { name: "add effect" });
    expect(within(popover).getByRole("searchbox", { name: "search effects" })).toBeInTheDocument();
    expect(within(popover).getByRole("button", { name: "plugin settings" })).toBeInTheDocument();
    fireEvent.click(await within(popover).findByRole("button", { name: "compressor" }));
    expect(picked).toEqual(["compressor"]);
  });
});

// A plugin installed in several formats is listed once, in the preferred format (by default),
// or once per format; hidden plugins aren't listed.
describe("DeviceLibrary: one format per plugin, hidden plugins", () => {
  it("lists each plugin once by default, CLAP first; the preferred format and hide rules apply", async () => {
    const t = new DesktopFake();
    t.plugins = [VERB, VST_VERB, VST2_VERB, EQ];
    await renderWith(t, <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    fireEvent.click(screen.getByRole("button", { name: "plugins" }));
    await waitFor(() => expect(names(list)).toEqual(["Air EQOtherclap", "VerbAcmeclap"]));
    act(() => usePluginVisibility.getState().setPreferred("Vst3"));
    expect(names(list)).toEqual(["Air EQOtherclap", "VerbAcmevst3"]);
    // Its VST3 version hidden: the next format in line (CLAP).
    act(() => void usePluginVisibility.getState().hide({ name: "Verb", vendor: "Acme", format: "Vst3" }));
    expect(names(list)).toEqual(["Air EQOtherclap", "VerbAcmeclap"]);
    // Every format of a plugin hidden: gone.
    act(() => void usePluginVisibility.getState().hide({ name: "Air EQ", vendor: "Other", format: null }));
    expect(names(list)).toEqual(["VerbAcmeclap"]);
    // Off: every format, the hidden ones still out.
    act(() => usePluginVisibility.getState().setDedupe(false));
    expect(names(list)).toEqual(["VerbAcmeclap", "VerbAcmevst2"]);
  });

  it("a row's right-click hides the plugin (every format or this one), undoable from the toast", async () => {
    const t = new DesktopFake();
    t.plugins = [VERB, VST_VERB, EQ];
    await renderWith(t, <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    fireEvent.click(screen.getByRole("button", { name: "plugins" }));
    await waitFor(() => expect(names(list)).toEqual(["Air EQOtherclap", "VerbAcmeclap"]));
    fireEvent.contextMenu(list.getByRole("button", { name: "Verb" }));
    const menu = useContextMenuStore.getState().menu!.items;
    const item = (label: string) => menu.find((i) => i !== "separator" && i.label === label) as Exclude<(typeof menu)[number], "separator">;
    expect(item("hide Verb")).toBeTruthy();
    act(() => item("hide this clap version").onSelect());
    expect(usePluginVisibility.getState().rules).toEqual([{ name: "Verb", vendor: "Acme", format: "Clap" }]);
    expect(names(list)).toEqual(["Air EQOtherclap", "VerbAcmevst3"]);
    const toast = useNotices.getState().notices.at(-1)!;
    expect(toast.message).toBe("Verb is hidden from the plugin list.");
    act(() => toast.action!.onSelect());
    expect(names(list)).toEqual(["Air EQOtherclap", "VerbAcmeclap"]);
  });

  it("the resolver's and Replace plugin's lists keep every format", async () => {
    act(() => void usePluginVisibility.getState().hide({ name: "Verb", vendor: "Acme", format: null }));
    await renderWith(new DesktopFake(), <DeviceLibrary pluginsOnly plugins={[VERB, VST_VERB]} onPick={() => undefined} />);
    const list = await rows("plugins");
    await waitFor(() => expect(names(list)).toEqual(["VerbAcmeclap", "VerbAcmevst3"]));
  });
});

// Every format listed (one format per plugin turned off); format toggles filter.
describe("DeviceLibrary plugin formats", () => {
  const formatGroup = () => screen.getByRole("group", { name: "plugin formats" });
  const toggle = (label: string) => within(formatGroup()).getByRole("button", { name: label });
  beforeEach(() => usePluginVisibility.getState().setDedupe(false));

  it("lists a plugin once per format, each with its chip; the toggles (the formats present) filter the list and its search", async () => {
    const t = new DesktopFake();
    t.plugins = [VERB, VST_VERB, VST2_VERB, EQ, AU_DELAY];
    await renderWith(t, <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    await list.findByText("Air EQ");
    fireEvent.click(screen.getByRole("button", { name: "plugins" }));
    expect(names(list)).toEqual(["Air EQOtherclap", "AUDelayAppleau", "VerbAcmeclap", "VerbAcmevst2", "VerbAcmevst3"]);
    // AU is macOS-only: no toggle here (jsdom isn't a Mac), and its plugins stay listed.
    expect(
      within(formatGroup())
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["clap", "vst2", "vst3"]);
    for (const f of ["clap", "vst2", "vst3"]) expect(toggle(f)).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(toggle("vst2"));
    expect(toggle("vst2")).toHaveAttribute("aria-pressed", "false");
    expect(names(list)).toEqual(["Air EQOtherclap", "AUDelayAppleau", "VerbAcmeclap", "VerbAcmevst3"]);
    // Search applies within the shown formats (a hidden format's name doesn't bring it back).
    const search = screen.getByRole("searchbox", { name: "search effects" });
    fireEvent.change(search, { target: { value: "verb" } });
    expect(names(list)).toEqual(["VerbAcmeclap", "VerbAcmevst3"]);
    fireEvent.change(search, { target: { value: "vst2" } });
    expect(list.getByText("no match.")).toBeInTheDocument();
    fireEvent.change(search, { target: { value: "" } });

    // At least one stays on: the last one shown can't be turned off.
    fireEvent.click(toggle("clap"));
    expect(names(list)).toEqual(["AUDelayAppleau", "VerbAcmevst3"]);
    expect(toggle("vst3")).toBeDisabled();
    expect(toggle("clap")).toBeEnabled();

    // The factory source has no plugins: no toggles.
    fireEvent.click(screen.getByRole("button", { name: "factory" }));
    expect(screen.queryByRole("group", { name: "plugin formats" })).toBeNull();
  });

  it("remembers the formats turned off (local settings)", async () => {
    const desktop = () => {
      const t = new DesktopFake();
      t.plugins = [VERB, VST_VERB, VST2_VERB];
      return t;
    };
    await renderWith(desktop(), <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    let list = await rows("effects");
    await waitFor(() => expect(names(list)).toContain("VerbAcmevst2"));
    fireEvent.click(toggle("vst2"));
    expect(JSON.parse(localStorage.getItem("eth-device-library")!)).toMatchObject({ hiddenFormats: ["Vst2"] });
    // A new session reads it back.
    vi.resetModules();
    const fresh = await import("./libraryStats");
    expect([...fresh.useLibraryStats.getState().hiddenFormats]).toEqual(["Vst2"]);
    cleanup();
    transport?.dispose();

    await renderWith(desktop(), <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    list = await rows("effects");
    await waitFor(() => expect(names(list)).toContain("VerbAcmevst3"));
    expect(names(list)).not.toContain("VerbAcmevst2");
    expect(toggle("vst2")).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(toggle("vst2"));
    expect(names(list)).toContain("VerbAcmevst2");
    expect(JSON.parse(localStorage.getItem("eth-device-library")!)).toMatchObject({ hiddenFormats: [] });
  });

  it("no toggles for a single format; a remembered setting hiding every present format shows them all", async () => {
    const t = new DesktopFake();
    t.plugins = [VERB, EQ];
    act(() => useLibraryStats.getState().setFormatShown("Clap", false));
    await renderWith(t, <DeviceLibrary category="AudioEffect" onPick={() => undefined} />);
    const list = await rows("effects");
    await list.findByText("Verb");
    expect(screen.queryByRole("group", { name: "plugin formats" })).toBeNull();

    cleanup();
    transport?.dispose();
    act(() => useLibraryStats.getState().setFormatShown("Vst3", false));
    await renderWith(new DesktopFake(), <DeviceLibrary pluginsOnly plugins={[VERB, VST_VERB]} onPick={() => undefined} />);
    const plugins = await rows("plugins");
    expect(names(plugins)).toEqual(["VerbAcmeclap", "VerbAcmevst3"]);
    expect(toggle("clap")).toHaveAttribute("aria-pressed", "true");
    expect(toggle("vst3")).toHaveAttribute("aria-pressed", "true");
  });

  it("the format toggles show AU on macOS only", () => {
    expect(platformFormats(true)).toEqual(["Clap", "Vst2", "Vst3", "Au", "Webclap"]);
    expect(platformFormats(false)).toEqual(["Clap", "Vst2", "Vst3", "Webclap"]);
  });

  it("lists both VST2 plugins sharing a uniqueID, and the insert names the one picked", async () => {
    const twin = { ...plugin(VST2_VERB.id, "Other Verb", "AudioEffect", "Elsewhere", "Vst2"), path: "/vst/other.so" };
    // The same plugin found in two folders loads the same: one row.
    const copy = { ...VST2_VERB, path: "/vst/copy/Verb.so" };
    const picked: LibraryDevicePayload[] = [];
    await renderWith(new DesktopFake(), <DeviceLibrary pluginsOnly plugins={[VST2_VERB, twin, copy, VERB]} onPick={(p) => picked.push(p)} />);
    const list = await rows("plugins");
    expect(names(list)).toEqual(["Other VerbElsewherevst2", "VerbAcmeclap", "VerbAcmevst2"]);
    const keys = list.getAllByRole("listitem").map((li) => li.querySelector<HTMLElement>("[data-key]")!.dataset.key);
    expect(new Set(keys).size).toBe(3);

    fireEvent.click(list.getByRole("button", { name: "Other Verb" }));
    fireEvent.click(list.getAllByRole("button", { name: "Verb" })[1]!);
    fireEvent.click(list.getAllByRole("button", { name: "Verb" })[0]!);
    expect(picked.map(deviceSpec)).toEqual([
      { type: "Plugin", plugin_id: VST2_VERB.id, format: "Vst2", sandboxed: null, name: "Other Verb" },
      { type: "Plugin", plugin_id: VST2_VERB.id, format: "Vst2", sandboxed: null, name: "Verb" },
      // No collision: no name sent.
      { type: "Plugin", plugin_id: VERB.id, format: "Clap", sandboxed: null },
    ]);
    // Favourites and usage tell them apart too.
    expect(picked.map(libraryKey)).toEqual([`plugin:Vst2:${VST2_VERB.id}:Other Verb`, `plugin:Vst2:${VST2_VERB.id}:Verb`, `plugin:Clap:${VERB.id}`]);
  });
});
