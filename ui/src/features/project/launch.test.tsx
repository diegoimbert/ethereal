// base-131: launch with no project open (the project screen first), "Reopen last project
// on launch" (only after a clean close), and "Open without plugins" (safe mode).
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Project } from "@/generated";
import { resetStores } from "@/features/transport-bar/testUtils";
import { useContextMenuStore } from "@/kit";
import { useProjectStore } from "@/state";
import { MockTransport, TransportProvider, type EngineTransport } from "@/transport";
import { ProjectMenu } from "./index";
import { LAST_PROJECT_KEY } from "@/features/versions/session";
import { REOPEN_KEY, useLaunchPrefs } from "./launch";
import { useProjectScreen } from "./screenStore";

const store = () => useProjectStore.getState();

/** A host that connects with nothing open, like every real host now (the mock opens its demo). */
class NothingOpen implements EngineTransport {
  readonly kind = "mock" as const;
  constructor(readonly inner: MockTransport) {}
  async connect(): Promise<Project | null> {
    await this.inner.connect();
    return null;
  }
  send: EngineTransport["send"] = (c, o) => this.inner.send(c, o);
  onEvent: EngineTransport["onEvent"] = (l) => this.inner.onEvent(l);
  subscribePlayhead: EngineTransport["subscribePlayhead"] = (l) => this.inner.subscribePlayhead(l);
  subscribeMeters: EngineTransport["subscribeMeters"] = (l) => this.inner.subscribeMeters(l);
  dispose() {
    this.inner.dispose();
  }
}

function launchWith(setup?: (mock: MockTransport) => void) {
  const mock = new MockTransport({ timers: "manual", seed: 7 });
  setup?.(mock);
  render(
    <TransportProvider transport={new NothingOpen(mock)}>
      <ProjectMenu />
    </TransportProvider>,
  );
  return mock;
}

/** A stored project other than the mock's open demo. */
async function otherProject(mock: MockTransport) {
  const reply = await mock.send({ domain: "Project", command: { type: "List" } });
  if (reply.type !== "Projects") throw new Error("no list");
  return reply.projects.find((p) => p.id !== mock.snapshot().id)!;
}

beforeEach(() => {
  localStorage.clear();
  useLaunchPrefs.setState({ reopenLast: false });
});

afterEach(() => {
  resetStores();
  useProjectScreen.setState({ open: false, launchPending: false });
  localStorage.clear();
});

describe("launch", () => {
  it("opens nothing: the project screen comes first", async () => {
    launchWith();
    const dialog = await screen.findByRole("dialog", { name: "Projects" });
    expect(store().project).toBeNull();
    expect(screen.getByTestId("project-name")).toHaveTextContent("No project");
    expect(within(dialog).queryByRole("button", { name: "Continue" })).toBeNull();
    expect(within(dialog).getByRole("button", { name: "New project" })).toBeInTheDocument();
  });

  it("reopens the last project when asked to and the last session closed cleanly", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    const last = await otherProject(mock);
    localStorage.setItem(LAST_PROJECT_KEY, last.id);
    act(() => useLaunchPrefs.getState().setReopenLast(true));
    expect(localStorage.getItem(REOPEN_KEY)).toBe("1");
    render(
      <TransportProvider transport={new NothingOpen(mock)}>
        <ProjectMenu />
      </TransportProvider>,
    );
    await waitFor(() => expect(store().project?.id).toBe(last.id));
    expect(useProjectScreen.getState().open).toBe(false);
  });

  it("never reopens after a crash, even when asked to", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    const last = await otherProject(mock);
    localStorage.setItem(LAST_PROJECT_KEY, last.id);
    useLaunchPrefs.setState({ reopenLast: true });
    // The session that had it open never closed cleanly.
    mock.versions.simulateCrash(last.id);
    render(
      <TransportProvider transport={new NothingOpen(mock)}>
        <ProjectMenu />
      </TransportProvider>,
    );
    await screen.findByRole("dialog", { name: "Projects" });
    expect(store().project).toBeNull();
  });

  it("opens a project without plugins from Recents; Load plugins leaves safe mode", async () => {
    const mock = launchWith();
    const other = await otherProject(mock);
    const dialog = await screen.findByRole("dialog", { name: "Projects" });
    const more = await within(dialog).findByRole("button", { name: `More actions for ${other.name}` });
    await waitFor(() => expect(more).toBeEnabled());
    fireEvent.click(more);
    const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "Open without plugins");
    if (!item || item === "separator") throw new Error("no Open without plugins item");
    act(() => item.onSelect());
    await waitFor(() => expect(store().project?.id).toBe(other.id));
    expect(store().safe).toBe(true);
    const banner = await screen.findByTestId("safe-mode-banner");
    expect(banner).toHaveTextContent("Plugins disabled (safe mode)");
    expect(store().dirty).toBe(false);
    fireEvent.click(within(banner).getByRole("button", { name: "Load plugins" }));
    await waitFor(() => expect(screen.queryByTestId("safe-mode-banner")).toBeNull());
    expect(store().safe).toBe(false);
  });
});
