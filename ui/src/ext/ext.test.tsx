import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useReducer } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Clip } from "@/generated";
import { buildCommands } from "@/app/shell/commands";
import { ExtensionRoots } from "@/app/ExtensionRoots";
import { PanelBoundary } from "@/app/PanelBoundary";
import { LeftRail } from "@/app/shell/LeftRail";
import { openSettings, AudioSettingsDialog } from "@/features/audio-settings";
import { badgesOf } from "@/features/arrangement/gl/layers/clipBadges";
import { DesktopBandFrame, resetDesktopBandDismissal } from "@/features/desktop-band";
import { menuLabels } from "@/features/desktop-menu";
import { ProjectMenu } from "@/features/project";
import { guardLeave } from "@/features/project/leaveGuard";
import { EngineErrorScreen } from "@/features/startup";
import { TransportBar } from "@/features/transport-bar";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useShellStore } from "@/state/shell";
import { cmd } from "@/transport";
import { MockTransport } from "@/transport/mock/MockTransport";
import { contribute, contributions, emit, resetExtensionsForTests } from "./index";

beforeEach(() => resetExtensionsForTests());
afterEach(() => {
  cleanup();
  resetExtensionsForTests();
  resetStores();
  resetDesktopBandDismissal();
});

describe("the registry", () => {
  it("keeps contributions sorted by order, then as contributed, and takes them back", () => {
    const off = contribute("app.event", () => undefined, 5);
    const a = () => undefined;
    const b = () => undefined;
    contribute("app.event", a);
    contribute("app.event", b, -1);
    expect(contributions("app.event").slice(0, 2)).toEqual([b, a]);
    off();
    expect(contributions("app.event")).toEqual([b, a]);
    // An empty point is stable (components can read it every render).
    expect(contributions("web.route")).toBe(contributions("web.route"));
  });

  it("emits to every listener; one that throws doesn't stop the others", () => {
    const seen: string[] = [];
    vi.spyOn(console, "warn").mockImplementation(() => undefined);
    contribute("app.event", () => {
      throw new Error("x");
    });
    contribute("app.event", (e) => void seen.push(e.type));
    emit("app.event", { type: "project_opened" });
    expect(seen).toEqual(["project_opened"]);
  });
});

describe("shell points", () => {
  it("mounts a contributed root in its slot, in an error boundary that names it", () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const crashes: string[] = [];
    contribute("app.crash", (c) => void crashes.push(c.panel));
    contribute("shell.root", { name: "ok root", Component: () => <p>hello</p> });
    contribute("shell.root", {
      name: "bad root",
      label: () => "the bad root",
      Component: () => {
        throw new Error("boom");
      },
    });
    contribute("shell.root", { name: "top", slot: "top-bar", dataSlot: "extra", Component: () => <span>top</span> });
    render(
      <>
        <ExtensionRoots shell="editor" slot="overlay" />
        <ExtensionRoots shell="editor" slot="top-bar" />
        <ExtensionRoots shell="mobile" slot="overlay" />
      </>,
    );
    expect(screen.getAllByText("hello")).toHaveLength(1);
    expect(screen.getByText(/the bad root/)).toBeInTheDocument();
    expect(crashes).toEqual(["bad root"]);
    expect(screen.getByText("top").closest("[data-slot]")?.getAttribute("data-slot")).toBe("extra");
  });

  it("a crash card shows the extensions' actions", () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    contribute("app.crashActions", ({ fullWindow }) => <button>report {fullWindow ? "app" : "panel"}</button>);
    const Bad = () => {
      throw new Error("boom");
    };
    render(
      <PanelBoundary name="library">
        <Bad />
      </PanelBoundary>,
    );
    expect(screen.getByRole("button", { name: "report panel" })).toBeInTheDocument();
    cleanup();
    render(<EngineErrorScreen error={new Error("no engine")} />);
    expect(screen.getByRole("button", { name: "report app" })).toBeInTheDocument();
  });

  it("a left rail tab shows while its extension says so, and closes its pane when it stops", async () => {
    let visible = true;
    const listeners = new Set<() => void>();
    const useVisible = () => {
      const [, force] = useStateTick();
      listeners.add(force);
      return visible;
    };
    contribute("shell.leftTab", { id: "chat", label: () => "chat", icon: <span />, useVisible, isVisible: () => visible, Component: () => <p>the chat</p> });
    render(<LeftRail />);
    fireEvent.click(screen.getByRole("button", { name: "chat" }));
    expect(useShellStore.getState().left).toMatchObject({ open: true, tab: "chat" });
    expect(buildCommands(null, []).some((c) => c.id === "panel:chat")).toBe(true);
    visible = false;
    act(() => listeners.forEach((l) => l()));
    await waitFor(() => expect(screen.queryByRole("button", { name: "chat" })).toBeNull());
    expect(useShellStore.getState().left.open).toBe(false);
    expect(buildCommands(null, []).some((c) => c.id === "panel:chat")).toBe(false);
  });

  it("the palette lists the extensions' commands", () => {
    const run = vi.fn();
    contribute("palette.commands", ({ transport }) => (transport ? [] : [{ id: "x:hello", group: "Help", label: "hello", run }]));
    const entry = buildCommands(null, []).find((c) => c.id === "x:hello");
    entry?.run();
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("the desktop menu labels the extensions' items", () => {
    contribute("desktopMenu.labels", () => ({ "app:report-issue": "report an issue…" }));
    expect(menuLabels().actions["app:report-issue"]).toBe("report an issue…");
    // This build handles no such item by itself.
    resetExtensionsForTests();
    expect(menuLabels().actions["app:report-issue"]).toBeUndefined();
  });
});

describe("settings", () => {
  it("a contributed tab (with or without an engine) and sections of the app's tabs", async () => {
    contribute("settings.tab", { id: "sync", label: () => "sync", noEngine: true, Component: () => <p>sync body</p> });
    contribute("settings.section", { tab: "general", noEngine: true, Component: () => <p>general extra</p> });
    render(<AudioSettingsDialog />);
    act(() => openSettings("general"));
    const dialog = await screen.findByRole("dialog", { name: "settings" });
    expect(within(dialog).getByText("general extra")).toBeInTheDocument();
    const tabs = within(dialog)
      .getAllByRole("tab")
      .map((t) => t.textContent);
    expect(tabs.indexOf("sync")).toBe(tabs.indexOf("advanced") - 1);
    fireEvent.click(within(dialog).getByRole("tab", { name: "sync" }));
    expect(within(dialog).getByText("sync body")).toBeInTheDocument();
    expect(within(dialog).queryByText("no engine connected.")).toBeNull();
  });
});

describe("projects", () => {
  it("the project menu shows the extensions' commands and submenus", async () => {
    const ran: string[] = [];
    contribute("project.commands", ({ project }) =>
      project
        ? [
            { id: "x:one", group: "Project", label: "export to x…", short: "x…", run: () => void ran.push("one") },
            { id: "x:two", group: "Project", label: "export to y…", short: "y…", run: () => void ran.push("two") },
          ]
        : [],
    );
    contribute("project.menu", { section: "export", item: { submenu: "export elsewhere", commands: ["x:one", "x:two"] } });
    await renderWithMock(<ProjectMenu />);
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    const item = await screen.findByRole("menuitem", { name: "export elsewhere" });
    fireEvent.mouseEnter(item.parentElement!);
    const sub = await screen.findByRole("menu", { name: "export elsewhere" });
    fireEvent.click(within(sub).getByRole("menuitem", { name: "y…" }));
    expect(ran).toEqual(["two"]);
  });

  it("a leave guard may ask first; without one the action runs", async () => {
    const action = vi.fn();
    expect(await guardLeave(action)).toBe(true);
    expect(action).toHaveBeenCalledTimes(1);
    contribute("project.leaveGuard", () => Promise.resolve(false));
    expect(await guardLeave(action)).toBe(false);
    expect(action).toHaveBeenCalledTimes(1);
  });
});

describe("views", () => {
  const audio = { id: "a", track: "t", start: 0, length: 4, content: { type: "Audio", media: "m", reversed: false } } as unknown as Clip;

  it("clip badges: an extension's, which may hide the missing one", () => {
    expect(badgesOf(audio, new Set(["m"]))).toEqual(["missing"]);
    contribute("clips.badge", { badge: () => ({ id: "loading", text: "loading", hatch: true, hidesMissing: true }), subscribe: () => () => undefined });
    expect(badgesOf(audio, new Set(["m"]))).toEqual(["loading"]);
  });

  it("the transport bar can't play while an extension says why", async () => {
    contribute("transport.blocked", () => "no engine here");
    await renderWithMock(<TransportBar />);
    const play = screen.getByRole("button", { name: "play" });
    expect(play).toBeDisabled();
    expect(play).toHaveAttribute("title", "no engine here");
  });

  it("the desktop band offers an extension's download page when the app didn't open", () => {
    vi.useFakeTimers();
    contribute("web.downloadUrl", () => "/download");
    render(
      <DesktopBandFrame show openDeepLink={() => undefined}>
        <div />
      </DesktopBandFrame>,
    );
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "open on desktop" }));
    act(() => vi.advanceTimersByTime(2000));
    expect(screen.getByRole("status").querySelector("a")?.getAttribute("href")).toBe("/download");
    vi.useRealTimers();
  });
});

describe("the mock engine", () => {
  it("answers an extension's domain, and leaves the rest unsupported", async () => {
    contribute("mock.domain", { domain: "Chat", handle: () => ({ type: "Unit" }) });
    const mock = new MockTransport({ timers: "manual" });
    await mock.connect();
    await expect(mock.send(cmd("Chat", { type: "Send", text: "hi" } as never))).resolves.toEqual({ type: "Unit" });
    mock.dispose();
  });
});

/** A re-render trigger for the visibility hook above. */
function useStateTick(): [number, () => void] {
  const [n, tick] = useReducer((x: number) => x + 1, 0);
  return [n, tick];
}
