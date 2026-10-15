// base-131: launch with no project open (the project screen first), "Reopen last project
// on launch" (only after a clean close), and "open without plugins" (safe mode).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Project } from "@/generated";
import { resetStores } from "@/features/transport-bar/testUtils";
import { useContextMenuStore } from "@/kit";
import { useProjectStore } from "@/state";
import { MockTransport, TransportProvider, type EngineTransport } from "@/transport";
import { ArrangementView } from "@/features/arrangement";
import { ProjectMenu } from "./index";
import { LAST_PROJECT_KEY } from "@/features/versions/session";
import { REOPEN_KEY, useLaunchPrefs } from "./launch";
import { RECENT_COUNT_KEY } from "./ProjectScreen";
import { useProjectScreen } from "./screenStore";
import { isDraft, resetDrafts } from "./drafts";

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

/** Let the project list catch up (a `List` round trip). */
async function flushList(mock: MockTransport) {
  await act(async () => {
    const r = await mock.send({ domain: "Project", command: { type: "List" } });
    if (r.type === "Projects") useProjectStore.getState().setProjects(r.projects);
  });
}

beforeEach(() => {
  localStorage.clear();
  useLaunchPrefs.setState({ reopenLast: false });
});

afterEach(() => {
  resetDrafts();
  resetStores();
  useProjectScreen.setState({ open: false, launchPending: false, launching: false });
  localStorage.clear();
});

describe("launch", () => {
  it("opens a draft (the starter project) and shows the project screen over it", async () => {
    launchWith();
    const dialog = await screen.findByRole("dialog", { name: "open project" });
    // The arranger is there behind the picker: a draft named after today, with a "Synth" track.
    await waitFor(() => expect(store().project?.settings.name).toBe(`${new Date().toLocaleDateString()} project`));
    expect(isDraft(store().project!.id)).toBe(true);
    expect(Object.values(store().project!.tracks).map((t) => t.name)).toContain("synth");
    expect(within(dialog).getByRole("button", { name: "new project" })).toBeInTheDocument();
    // The splash screen is text-only: the wordmark, no logo (the owner's call).
    expect(within(dialog).getByText("ethereal")).toBeInTheDocument();
    expect(within(dialog).queryByTestId("logo")).toBeNull();
  });

  it('keeps the arranger blank (no "Open a project") until launch has decided', async () => {
    useProjectScreen.setState({ launching: true });
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    render(
      <TransportProvider transport={new NothingOpen(mock)}>
        <ArrangementView />
        <ProjectMenu />
      </TransportProvider>,
    );
    expect(screen.queryByRole("button", { name: "open a project" })).toBeNull();
    await screen.findByRole("dialog", { name: "open project" });
    // The picker comes before the launch draft (#646): launch is done once that is open.
    await waitFor(() => expect(useProjectScreen.getState().launching).toBe(false));
  });

  it("dismissing the picker lands in a draft project (today's date) that only stays once changed", async () => {
    const mock = launchWith();
    const dialog = await screen.findByRole("dialog", { name: "open project" });
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(store().project?.settings.name).toBe(`${new Date().toLocaleDateString()} project`));
    const draft = store().project!.id;
    expect(isDraft(draft)).toBe(true);

    // Left untouched: opening another project deletes it.
    const other = await otherProject(mock);
    await act(async () => void (await mock.send({ domain: "Project", command: { type: "Open", id: other.id } })));
    await waitFor(() => expect(store().projects.some((p) => p.id === draft)).toBe(false));
    expect(isDraft(draft)).toBe(false);
  });

  it("a draft that gets changed is kept", async () => {
    const mock = launchWith();
    fireEvent.keyDown(await screen.findByRole("dialog", { name: "open project" }), { key: "Escape" });
    await waitFor(() => expect(store().project).not.toBeNull());
    const draft = store().project!.id;
    await act(async () => void (await mock.send({ domain: "Project", command: { type: "Rename", id: draft, name: "Kept" } })));
    await waitFor(() => expect(isDraft(draft)).toBe(false));
    const other = await otherProject(mock);
    await act(async () => void (await mock.send({ domain: "Project", command: { type: "Open", id: other.id } })));
    await flushList(mock);
    expect(store().projects.some((p) => p.id === draft)).toBe(true);
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
    await screen.findByRole("dialog", { name: "open project" });
    // Only the launch draft is open, not the crashed project.
    await waitFor(() => expect(store().project).not.toBeNull());
    expect(store().project!.id).not.toBe(last.id);
    expect(isDraft(store().project!.id)).toBe(true);
  });

  it("opens a project without plugins from Recents; Load plugins leaves safe mode", async () => {
    const mock = launchWith();
    const other = await otherProject(mock);
    const dialog = await screen.findByRole("dialog", { name: "open project" });
    const more = await within(dialog).findByRole("button", { name: `more actions for ${other.name}` });
    await waitFor(() => expect(more).toBeEnabled());
    fireEvent.click(more);
    const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "open without plugins");
    if (!item || item === "separator") throw new Error("no Open without plugins item");
    act(() => item.onSelect());
    await waitFor(() => expect(store().project?.id).toBe(other.id));
    expect(store().safe).toBe(true);
    const banner = await screen.findByTestId("safe-mode-banner");
    expect(banner).toHaveTextContent("plugins disabled (safe mode)");
    expect(store().dirty).toBe(false);
    fireEvent.click(within(banner).getByRole("button", { name: "load plugins" }));
    await waitFor(() => expect(screen.queryByTestId("safe-mode-banner")).toBeNull());
    expect(store().safe).toBe(false);
  });
});

/** The browser's engine (`kind: "wasm"`), still loading: `connect()` waits for `ready()`. */
class LoadingWasm implements EngineTransport {
  readonly kind = "wasm" as const;
  private readonly inner: NothingOpen;
  private release!: () => void;
  private readonly loaded = new Promise<void>((r) => (this.release = r));
  constructor(mock: MockTransport) {
    this.inner = new NothingOpen(mock);
  }
  async connect(): Promise<Project | null> {
    await this.loaded;
    return this.inner.connect();
  }
  ready() {
    this.release();
  }
  send: EngineTransport["send"] = (c, o) => this.inner.send(c, o);
  onEvent: EngineTransport["onEvent"] = (l) => this.inner.onEvent(l);
  subscribePlayhead: EngineTransport["subscribePlayhead"] = (l) => this.inner.subscribePlayhead(l);
  subscribeMeters: EngineTransport["subscribeMeters"] = (l) => this.inner.subscribeMeters(l);
  dispose() {
    this.inner.dispose();
  }
}

function launchLoading() {
  useProjectScreen.setState({ launching: true, launchShown: false });
  const engine = new LoadingWasm(new MockTransport({ timers: "manual", seed: 7 }));
  render(
    <TransportProvider transport={engine}>
      <ArrangementView />
      <ProjectMenu />
    </TransportProvider>,
  );
  return engine;
}

describe("launch before the engine is up (#646)", () => {
  afterEach(() => window.history.replaceState(null, "", "/"));

  it("shows the picker at once, its actions waiting for the engine, then enables them", async () => {
    localStorage.setItem(RECENT_COUNT_KEY, "3");
    const engine = launchLoading();
    // The first render, no engine yet: the picker, with its actions disabled and placeholder
    // rows (as many as the list had last time); nothing open behind it.
    const dialog = screen.getByRole("dialog", { name: "open project" });
    expect(within(dialog).getByTestId("project-screen-loading")).toHaveTextContent("loading engine…");
    expect(within(dialog).queryByTestId("logo")).toBeNull();
    expect(within(dialog).getByRole("button", { name: "new project" })).toBeDisabled();
    expect(within(dialog).getByTestId("project-list-skeleton").children).toHaveLength(3);
    expect(store().project).toBeNull();

    act(() => engine.ready());
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "new project" })).toBeEnabled());
    expect(within(dialog).queryByTestId("project-screen-loading")).toBeNull();
    expect(within(dialog).queryByTestId("project-list-skeleton")).toBeNull();
    // The launch draft behind it, the picker still up, the list's size kept for next time.
    expect(isDraft(store().project!.id)).toBe(true);
    expect(useProjectScreen.getState().open).toBe(true);
    await within(dialog).findByRole("list", { name: "stored projects" });
    await waitFor(() => expect(Number(localStorage.getItem(RECENT_COUNT_KEY))).toBeGreaterThan(0));
  });

  it("dismissed while loading, it stays closed: launch makes the draft behind it", async () => {
    const engine = launchLoading();
    fireEvent.keyDown(screen.getByRole("dialog", { name: "open project" }), { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "open project" })).toBeNull();
    act(() => engine.ready());
    await waitFor(() => expect(useProjectScreen.getState().launching).toBe(false));
    expect(isDraft(store().project!.id)).toBe(true);
    expect(useProjectScreen.getState().open).toBe(false);
  });

  it("waits for the engine on a project URL, after a crash or with a project to reopen", () => {
    const cases: Array<[string, () => void, () => void]> = [
      ["a project URL", () => window.history.replaceState(null, "", "/p/song/abc"), () => window.history.replaceState(null, "", "/")],
      ["a crash", () => localStorage.setItem("eth.session", "open"), () => localStorage.removeItem("eth.session")],
      [
        "reopen last",
        () => {
          localStorage.setItem(LAST_PROJECT_KEY, "abc");
          useLaunchPrefs.setState({ reopenLast: true });
        },
        () => useLaunchPrefs.setState({ reopenLast: false }),
      ],
    ];
    for (const [name, setup, undo] of cases) {
      setup();
      launchLoading();
      expect(screen.queryByRole("dialog", { name: "open project" }), name).toBeNull();
      expect(useProjectScreen.getState().open, name).toBe(false);
      cleanup();
      undo();
    }
  });
});
