import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, Event, PluginCommand, PluginEvent, PluginFolders, ReplyValue, ScanFailure } from "@/generated";
import { ContextMenuHost } from "@/kit";
import { pickOption } from "@/kit/testing";
import { useProjectStore } from "@/state";
import { Emitter, TransportProvider, type EngineTransport, type SendOptions, type Unsubscribe } from "@/transport";
import { MockTransport } from "@/transport/mock";
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
      { path: "/Library/Audio/Plug-Ins/clap", format: "Clap", exists: true },
      { path: "/Library/Audio/Plug-Ins/vst3", format: "Vst3", exists: false },
      { path: "/Library/Audio/Plug-Ins/vst", format: "Vst2", exists: true },
    ],
    folders: [],
  };
  /** The engine's failed targets (`ListScanFailures`). */
  failures: ScanFailure[] = [];
  /** Pending `RetryScan` replies, resolved by the test. */
  retries: ((reply: ReplyValue) => void)[] = [];

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
      case "ListScanFailures":
        return { type: "ScanFailures", failed: structuredClone(this.failures) };
      case "RetryScan":
        return new Promise((resolve) => this.retries.push(resolve));
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
      <ContextMenuHost />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  return t;
}

describe("PluginFoldersPanel", () => {
  it("lists system folders read-only, adds, re-filters and removes user folders (each rescans)", async () => {
    const t = await renderPanel(new FoldersFake());
    const system = await screen.findByRole("list", { name: "system folders" });
    expect(within(system).getByText("/Library/Audio/Plug-Ins/clap")).toBeInTheDocument();
    expect(within(system).getByText("not found")).toBeInTheDocument();
    expect(within(system).queryByRole("button")).toBeNull();
    expect(screen.getByText(/no folders added/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "add folder…" }));
    const mine = await screen.findByRole("list", { name: "your folders" });
    expect(within(mine).getByText("/Users/me/Plugins")).toBeInTheDocument();
    expect(t.sent.at(-1)).toEqual({ type: "AddFolder", path: "/Users/me/Plugins", format: null });
    // The engine rescans after a folder change: the panel shows it until ScanFinished.
    expect(screen.getByRole("button", { name: "scanning…" })).toBeDisabled();
    t.emit({ type: "ScanProgress", done: 2, total: 5, current: "/Users/me/Plugins/A.clap" });
    expect(screen.getByText("scanning 2/5")).toBeInTheDocument();
    t.emit({ type: "ScanFinished", plugins: 5, failed: [] });
    expect(screen.getByRole("button", { name: "rescan" })).toBeEnabled();

    pickOption(screen.getByRole("combobox", { name: "formats in /Users/me/Plugins" }), { value: "Vst3" });
    await waitFor(() => expect(t.sent.at(-1)).toEqual({ type: "AddFolder", path: "/Users/me/Plugins", format: "Vst3" }));
    t.emit({ type: "ScanFinished", plugins: 5, failed: [] });
    pickOption(screen.getByRole("combobox", { name: "formats in /Users/me/Plugins" }), { value: "Vst2" });
    await waitFor(() => expect(t.sent.at(-1)).toEqual({ type: "AddFolder", path: "/Users/me/Plugins", format: "Vst2" }));
    t.emit({ type: "ScanFinished", plugins: 5, failed: [] });

    fireEvent.click(await screen.findByRole("button", { name: "remove /Users/me/Plugins" }));
    await screen.findByText(/no folders added/);
    expect(t.sent.at(-1)).toEqual({ type: "RemoveFolder", path: "/Users/me/Plugins" });
  });

  it("toggles the system folders and runs incremental and full rescans", async () => {
    const t = await renderPanel(new FoldersFake());
    const toggle = await screen.findByRole("switch");
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() => expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "false"));
    expect(t.sent.at(-1)).toEqual({ type: "SetIncludeDefaults", include: false });
    expect(screen.getByText(/system folders are skipped/)).toBeInTheDocument();
    t.emit({ type: "ScanFinished", plugins: 0, failed: [] });

    fireEvent.click(screen.getByRole("button", { name: "rescan" }));
    expect(t.sent.at(-1)).toEqual({ type: "Rescan" });
    t.emit({ type: "ScanFinished", plugins: 0, failed: [{ path: "/p/bad.clap", message: "crashed" }] });
    expect(screen.getByText(/1 plugin failed to load/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "full rescan" }));
    expect(t.sent.at(-1)).toEqual({ type: "Rescan", full: true });
  });

  it("a dismissed folder dialog adds nothing", async () => {
    const t = await renderPanel(new FoldersFake());
    t.picked = null;
    fireEvent.click(await screen.findByRole("button", { name: "add folder…" }));
    await waitFor(() => expect(t.sent.map((c) => c.type).sort()).toEqual(["ListFolders", "ListScanFailures"]));
  });

  describe("failed plugins", () => {
    const kontakt = "C:\\Program Files\\Common Files\\vst3\\Kontakt 7.vst3";
    const failures = (): ScanFailure[] => [
      { path: kontakt, message: "scan timed out after 20s", format: "Vst3" },
      { path: "/p/bad.clap", message: "", format: "Clap" },
    ];
    const row = (path: string) =>
      within(screen.getByRole("list", { name: "failed plugins" }))
        .getByTitle(path)
        .closest("li")!;

    async function openFailed(): Promise<FoldersFake> {
      const t = new FoldersFake();
      t.failures = failures();
      await renderPanel(t);
      fireEvent.click(await screen.findByRole("button", { name: "failed plugins (2)…" }));
      await screen.findByRole("dialog", { name: "plugins that failed to load" });
      return t;
    }

    it("lists each failed plugin's file, format and reason (known before any scan)", async () => {
      await openFailed();
      const list = screen.getByRole("list", { name: "failed plugins" });
      expect(within(list).getAllByRole("listitem")).toHaveLength(2);
      expect(within(row(kontakt)).getByText("scan timed out after 20s")).toBeInTheDocument();
      expect(within(row(kontakt)).getByText("vst3")).toBeInTheDocument();
      // No reason from the engine: something neutral.
      expect(within(row("/p/bad.clap")).getByText("unknown error")).toBeInTheDocument();
      expect(within(row("/p/bad.clap")).getByText("clap")).toBeInTheDocument();
    });

    it("right-click → Retry… asks for a timeout (90 s by default) and rescans that plugin; a success removes the row", async () => {
      const t = await openFailed();
      fireEvent.contextMenu(row(kontakt));
      fireEvent.click(await screen.findByRole("menuitem", { name: "retry…" }));
      expect(screen.getByRole("spinbutton", { name: "timeout" })).toHaveValue("90");
      fireEvent.click(screen.getByRole("button", { name: "retry" }));
      await waitFor(() => expect(t.sent.at(-1)).toEqual({ type: "RetryScan", path: kontakt, format: "Vst3", timeout_ms: 90_000 }));
      // Busy while the engine scans; a second Retry is disabled meanwhile.
      expect(row(kontakt)).toHaveAttribute("aria-busy", "true");
      expect(within(row(kontakt)).getByText(/retrying…/)).toBeInTheDocument();
      fireEvent.contextMenu(row(kontakt));
      const retry = await screen.findByRole("menuitem", { name: "retry…" });
      expect(retry).toHaveAttribute("aria-disabled", "true");
      expect(retry).toHaveAccessibleDescription("already retrying");
      fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });

      await act(async () => {
        t.retries[0]!({ type: "ScanRetried", plugins: [], failure: null });
        await Promise.resolve();
      });
      await waitFor(() => expect(within(screen.getByRole("list", { name: "failed plugins" })).queryByTitle(kontakt)).toBeNull());
      expect(screen.getByRole("button", { name: "failed plugins (1)…" })).toBeInTheDocument();
      expect(t.retries).toHaveLength(1);
    });

    it("a retry that fails again keeps the row with the new reason", async () => {
      const t = await openFailed();
      fireEvent.contextMenu(row("/p/bad.clap"));
      fireEvent.click(await screen.findByRole("menuitem", { name: "retry…" }));
      // A longer timeout typed in the dialog is the one sent.
      const timeout = screen.getByRole("spinbutton", { name: "timeout" });
      fireEvent.change(timeout, { target: { value: "1500" } });
      fireEvent.blur(timeout);
      fireEvent.click(screen.getByRole("button", { name: "retry" }));
      await waitFor(() => expect(t.retries).toHaveLength(1));
      expect(t.sent.at(-1)).toEqual({ type: "RetryScan", path: "/p/bad.clap", format: "Clap", timeout_ms: 1_500_000 });
      await act(async () => {
        t.retries[0]!({
          type: "ScanRetried",
          plugins: [],
          failure: { path: "/p/bad.clap", message: "scanner crashed (exit code: 3)", format: "Clap" },
        });
        await Promise.resolve();
      });
      await waitFor(() => expect(within(row("/p/bad.clap")).getByText("scanner crashed (exit code: 3)")).toBeInTheDocument());
      expect(row("/p/bad.clap")).not.toHaveAttribute("aria-busy");
      expect(screen.getByRole("button", { name: "failed plugins (2)…" })).toBeInTheDocument();
    });
  });

  it("explains that plugins live in the desktop app elsewhere", () => {
    const mock = new MockTransport({ timers: "manual", seed: 1 });
    transport = mock;
    render(<PluginFoldersPanel transport={mock} />);
    expect(screen.getByText(/loaded by the desktop app/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "add folder…" })).toBeNull();
  });
});
