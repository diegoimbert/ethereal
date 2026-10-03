import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, Event, PluginCommand, PluginEvent, PluginFolders, ReplyValue } from "@/generated";
import { pickOption } from "@/kit/testing";
import { useProjectStore } from "@/state";
import { Emitter, MockTransport, TransportProvider, type EngineTransport, type SendOptions, type Unsubscribe } from "@/transport";
import { PluginFoldersPanel } from "./index";
import { usePluginStore } from "./pluginStore";

/** The desktop host's plugin-folder commands (state kept like the engine does) + a folder picker. */
class FoldersFake implements EngineTransport {
  readonly kind = "tauri" as const;
  readonly mock = new MockTransport({ timers: "manual", seed: 5 });
  readonly events = new Emitter<Event>();
  readonly sent: PluginCommand[] = [];
  picked: string | null = "/Users/me/Plugins";
  state: PluginFolders = {
    include_defaults: true,
    defaults: [
      { path: "/Library/Audio/Plug-Ins/CLAP", format: "Clap", exists: true },
      { path: "/Library/Audio/Plug-Ins/VST3", format: "Vst3", exists: false },
    ],
    folders: [],
  };

  connect() {
    return this.mock.connect();
  }
  async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    if (command.domain !== "Plugin") return this.mock.send(command, opts);
    const c = command.command;
    this.sent.push(c);
    const folders = (): ReplyValue => ({ type: "PluginFolders", folders: structuredClone(this.state) });
    switch (c.type) {
      case "ListFolders":
        return folders();
      case "AddFolder": {
        const f = this.state.folders.find((x) => x.path === c.path);
        if (f) f.format = c.format;
        else this.state.folders.push({ path: c.path, format: c.format });
        return folders();
      }
      case "RemoveFolder":
        this.state.folders = this.state.folders.filter((x) => x.path !== c.path);
        return folders();
      case "SetIncludeDefaults":
        this.state.include_defaults = c.include;
        return folders();
      case "List":
        return { type: "Plugins", plugins: [] };
      default:
        return { type: "Unit" };
    }
  }
  pickFolder(): Promise<string | null> {
    return Promise.resolve(this.picked);
  }
  onEvent(listener: (event: Event) => void): Unsubscribe {
    const a = this.mock.onEvent(listener);
    const b = this.events.on(listener);
    return () => {
      a();
      b();
    };
  }
  subscribePlayhead: EngineTransport["subscribePlayhead"] = (l) => this.mock.subscribePlayhead(l);
  subscribeMeters: EngineTransport["subscribeMeters"] = (l) => this.mock.subscribeMeters(l);
  dispose() {
    this.mock.dispose();
  }
  emit(event: PluginEvent) {
    act(() => this.events.emit({ type: "Plugin", event }));
  }
}

let transport: EngineTransport | undefined;
afterEach(() => {
  cleanup();
  transport?.dispose();
  transport = undefined;
  useProjectStore.getState().reset();
  usePluginStore.getState().reset();
});

async function renderPanel<T extends EngineTransport>(t: T): Promise<T> {
  transport = t;
  render(
    <TransportProvider transport={t}>
      <PluginFoldersPanel transport={t} />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  return t;
}

describe("PluginFoldersPanel", () => {
  it("lists system folders read-only, adds, re-filters and removes user folders (each rescans)", async () => {
    const t = await renderPanel(new FoldersFake());
    const system = await screen.findByRole("list", { name: "System folders" });
    expect(within(system).getByText("/Library/Audio/Plug-Ins/CLAP")).toBeInTheDocument();
    expect(within(system).getByText("not found")).toBeInTheDocument();
    expect(within(system).queryByRole("button")).toBeNull();
    expect(screen.getByText(/No folders added/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Add folder…" }));
    const mine = await screen.findByRole("list", { name: "Your folders" });
    expect(within(mine).getByText("/Users/me/Plugins")).toBeInTheDocument();
    expect(t.sent.at(-1)).toEqual({ type: "AddFolder", path: "/Users/me/Plugins", format: null });
    // The engine rescans after a folder change: the panel shows it until ScanFinished.
    expect(screen.getByRole("button", { name: "Scanning…" })).toBeDisabled();
    t.emit({ type: "ScanProgress", done: 2, total: 5, current: "/Users/me/Plugins/A.clap" });
    expect(screen.getByText("Scanning 2/5")).toBeInTheDocument();
    t.emit({ type: "ScanFinished", plugins: 5, failed: [] });
    expect(screen.getByRole("button", { name: "Rescan" })).toBeEnabled();

    pickOption(screen.getByRole("combobox", { name: "Formats in /Users/me/Plugins" }), { value: "Vst3" });
    await waitFor(() => expect(t.sent.at(-1)).toEqual({ type: "AddFolder", path: "/Users/me/Plugins", format: "Vst3" }));
    t.emit({ type: "ScanFinished", plugins: 5, failed: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Remove /Users/me/Plugins" }));
    await screen.findByText(/No folders added/);
    expect(t.sent.at(-1)).toEqual({ type: "RemoveFolder", path: "/Users/me/Plugins" });
  });

  it("toggles the system folders and runs incremental and full rescans", async () => {
    const t = await renderPanel(new FoldersFake());
    const toggle = await screen.findByRole("switch");
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() => expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "false"));
    expect(t.sent.at(-1)).toEqual({ type: "SetIncludeDefaults", include: false });
    expect(screen.getByText(/System folders are skipped/)).toBeInTheDocument();
    t.emit({ type: "ScanFinished", plugins: 0, failed: [] });

    fireEvent.click(screen.getByRole("button", { name: "Rescan" }));
    expect(t.sent.at(-1)).toEqual({ type: "Rescan" });
    t.emit({ type: "ScanFinished", plugins: 0, failed: [{ path: "/p/bad.clap", message: "crashed" }] });
    expect(screen.getByText(/1 plugin failed to load/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Full rescan" }));
    expect(t.sent.at(-1)).toEqual({ type: "Rescan", full: true });
  });

  it("a dismissed folder dialog adds nothing", async () => {
    const t = await renderPanel(new FoldersFake());
    t.picked = null;
    fireEvent.click(await screen.findByRole("button", { name: "Add folder…" }));
    await waitFor(() => expect(t.sent.map((c) => c.type)).toEqual(["ListFolders"]));
  });

  it("explains that plugins live in the desktop app elsewhere", () => {
    const mock = new MockTransport({ timers: "manual", seed: 1 });
    transport = mock;
    render(<PluginFoldersPanel transport={mock} />);
    expect(screen.getByText(/loaded by the desktop app/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add folder…" })).toBeNull();
  });
});
