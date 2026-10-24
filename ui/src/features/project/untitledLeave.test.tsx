// Leaving an untitled project (changed, never named): keep or discard it (#233).
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project } from "@/generated";
import { QuitRoot, setQuitBridge, useQuitConfirm, type QuitBridge } from "@/features/quit";
import { resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { newId, TransportProvider, type EngineTransport } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { isDraft, isUntitled, resetDrafts } from "./drafts";
import { ProjectMenu } from "./index";
import { useLaunchPrefs } from "./launch";
import { guardLeave } from "./leaveGuard";
import { binOpenProject } from "./projectCommands";
import { useProjectScreen } from "./screenStore";

const store = () => useProjectStore.getState();

/** A host that connects with nothing open (as every real host): launch makes a draft. */
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

function quitBridgeMock() {
  const handlers = new Set<() => void>();
  const bridge = {
    onQuitRequested: vi.fn(async (h: () => void) => {
      handlers.add(h);
      return () => handlers.delete(h);
    }),
    onCloseRequested: vi.fn(async () => () => undefined),
    setGuard: vi.fn(async () => {}),
    quit: vi.fn(async () => {}),
    cancel: vi.fn(async () => {}),
  } satisfies QuitBridge;
  return { bridge, requestQuit: () => act(() => handlers.forEach((h) => h())) };
}

/** Launch, dismiss the picker (a draft opens) and change it: an untitled project. */
async function untitledProject(withQuit?: QuitBridge) {
  const mock = new MockTransport({ timers: "manual", seed: 7 });
  const send = vi.spyOn(mock, "send");
  if (withQuit) setQuitBridge(withQuit);
  render(
    <TransportProvider transport={new NothingOpen(mock)}>
      <ProjectMenu />
      {withQuit && <QuitRoot />}
    </TransportProvider>,
  );
  fireEvent.keyDown(await screen.findByRole("dialog", { name: "open project" }), { key: "Escape" });
  await waitFor(() => expect(store().project).not.toBeNull());
  const id = store().project!.id;
  await act(
    async () =>
      void (await mock.send({ domain: "Track", command: { type: "Create", id: newId(), kind: "Midi", name: null, color: null, parent: null, before: null } })),
  );
  await waitFor(() => expect(isUntitled(id)).toBe(true));
  expect(isDraft(id)).toBe(false);
  return { mock, send, id, name: store().project!.settings.name };
}

/** A stored project other than the open one. */
async function other(mock: MockTransport, open: string) {
  const reply = await mock.send({ domain: "Project", command: { type: "List" } });
  if (reply.type !== "Projects") throw new Error("no list");
  return reply.projects.find((p) => p.id !== open)!;
}

async function stored(mock: MockTransport, id: string) {
  const reply = await mock.send({ domain: "Project", command: { type: "List" } });
  return reply.type === "Projects" && reply.projects.some((p) => p.id === id);
}

const prompt = () => screen.findByRole("dialog", { name: "discard this untitled project?" });

beforeEach(() => {
  localStorage.clear();
  useLaunchPrefs.setState({ reopenLast: false });
});

afterEach(() => {
  setQuitBridge(undefined);
  useQuitConfirm.setState({ open: false });
  resetDrafts();
  resetStores();
  useProjectScreen.setState({ open: false, launchPending: false, launching: false });
  localStorage.clear();
});

describe("leaving an untitled project", () => {
  it("opening another project from the project screen asks; discard deletes it", async () => {
    const { mock, id, name } = await untitledProject();
    const next = await other(mock, id);
    act(() => useProjectScreen.setState({ open: true, mode: "home" }));
    const picker = await screen.findByRole("dialog", { name: "open project" });
    fireEvent.click(within(picker).getByRole("button", { name: `open ${next.name}` }));
    const dialog = await prompt();
    expect(dialog).toHaveTextContent(name);
    fireEvent.click(within(dialog).getByRole("button", { name: "discard" }));
    await waitFor(() => expect(store().project?.id).toBe(next.id));
    await waitFor(async () => expect(await stored(mock, id)).toBe(false));
    expect(isUntitled(id)).toBe(false);
  });

  it("a new project from the welcome screen asks too", async () => {
    const { mock, id } = await untitledProject();
    act(() => useProjectScreen.setState({ open: true, mode: "home" }));
    fireEvent.click(await screen.findByTestId("welcome-new-project"));
    fireEvent.click(within(await prompt()).getByRole("button", { name: "discard" }));
    await waitFor(() => expect(store().project?.id).not.toBe(id));
    expect(isDraft(store().project!.id)).toBe(true);
    await waitFor(async () => expect(await stored(mock, id)).toBe(false));
  });

  it("keep with a name renames it and keeps it, then leaves", async () => {
    const { mock, id } = await untitledProject();
    const next = await other(mock, id);
    const left = guardLeave(() => mock.send({ domain: "Project", command: { type: "Open", id: next.id } }));
    const dialog = await prompt();
    fireEvent.change(within(dialog).getByRole("textbox", { name: "project name" }), { target: { value: "my song" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "keep" }));
    await expect(left).resolves.toBe(true);
    await waitFor(() => expect(store().project?.id).toBe(next.id));
    const list = await mock.send({ domain: "Project", command: { type: "List" } });
    expect(list.type === "Projects" && list.projects.find((p) => p.id === id)?.name).toBe("my song");
    expect(isUntitled(id)).toBe(false);
  });

  it("keep without a name keeps it as it is (and doesn't ask again)", async () => {
    const { mock, id, name } = await untitledProject();
    const next = await other(mock, id);
    void guardLeave(() => mock.send({ domain: "Project", command: { type: "Open", id: next.id } }));
    fireEvent.click(within(await prompt()).getByRole("button", { name: "keep" }));
    await waitFor(() => expect(store().project?.id).toBe(next.id));
    const list = await mock.send({ domain: "Project", command: { type: "List" } });
    expect(list.type === "Projects" && list.projects.find((p) => p.id === id)?.name).toBe(name);
    expect(isUntitled(id)).toBe(false);
  });

  it("cancel stays", async () => {
    const { mock, id } = await untitledProject();
    const action = vi.fn();
    const left = guardLeave(action);
    fireEvent.click(within(await prompt()).getByRole("button", { name: "cancel" }));
    await expect(left).resolves.toBe(false);
    expect(action).not.toHaveBeenCalled();
    expect(store().project?.id).toBe(id);
    expect(await stored(mock, id)).toBe(true);
    expect(isUntitled(id)).toBe(true);
  });

  it("discard keeps it when the action doesn't leave after all (a dismissed file picker)", async () => {
    const { mock, id } = await untitledProject();
    void guardLeave(() => Promise.resolve(null));
    fireEvent.click(within(await prompt()).getByRole("button", { name: "discard" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "discard this untitled project?" })).toBeNull());
    await act(async () => {});
    expect(await stored(mock, id)).toBe(true);
    expect(isUntitled(id)).toBe(true);
  });

  it("discard waits for a switch that comes after the action (an import, joining a session)", async () => {
    const { mock, id } = await untitledProject();
    const next = await other(mock, id);
    void guardLeave(() => Promise.resolve());
    fireEvent.click(within(await prompt()).getByRole("button", { name: "discard" }));
    await act(async () => {});
    expect(await stored(mock, id)).toBe(true);
    await act(async () => void (await mock.send({ domain: "Project", command: { type: "Open", id: next.id } })));
    await waitFor(async () => expect(await stored(mock, id)).toBe(false));
  });

  it("an edit after discard calls it off", async () => {
    const { mock, id } = await untitledProject();
    const next = await other(mock, id);
    void guardLeave(() => Promise.resolve());
    fireEvent.click(within(await prompt()).getByRole("button", { name: "discard" }));
    await act(async () => {});
    await act(
      async () =>
        void (await mock.send({
          domain: "Track",
          command: { type: "Create", id: newId(), kind: "Audio", name: null, color: null, parent: null, before: null },
        })),
    );
    await act(async () => void (await mock.send({ domain: "Project", command: { type: "Open", id: next.id } })));
    await act(async () => {});
    expect(await stored(mock, id)).toBe(true);
  });

  it("a named project leaves without asking", async () => {
    const { mock, id } = await untitledProject();
    await act(async () => void (await mock.send({ domain: "Project", command: { type: "Rename", id, name: "named" } })));
    await waitFor(() => expect(isUntitled(id)).toBe(false));
    const action = vi.fn();
    await expect(guardLeave(action)).resolves.toBe(true);
    expect(action).toHaveBeenCalledOnce();
    expect(screen.queryByRole("dialog", { name: "discard this untitled project?" })).toBeNull();
  });

  it("an untouched draft is not asked about (left, it is deleted silently, as before)", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    render(
      <TransportProvider transport={new NothingOpen(mock)}>
        <ProjectMenu />
      </TransportProvider>,
    );
    fireEvent.keyDown(await screen.findByRole("dialog", { name: "open project" }), { key: "Escape" });
    await waitFor(() => expect(store().project).not.toBeNull());
    const draft = store().project!.id;
    const next = await other(mock, draft);
    await act(async () => void (await guardLeave(() => mock.send({ domain: "Project", command: { type: "Open", id: next.id } }))));
    expect(screen.queryByRole("dialog", { name: "discard this untitled project?" })).toBeNull();
    await waitFor(async () => expect(await stored(mock, draft)).toBe(false));
  });

  it("moving the open untitled project to the bin doesn't ask (it is kept there)", async () => {
    const { mock, id } = await untitledProject();
    act(() => binOpenProject(new NothingOpen(mock), id, "Untitled"));
    await waitFor(() => expect(store().project?.id).not.toBe(id));
    expect(screen.queryByRole("dialog", { name: "discard this untitled project?" })).toBeNull();
  });
});

describe("quitting with an untitled project (desktop)", () => {
  it("asks keep or discard instead of the quit question; discard & quit discards it, then quits", async () => {
    const shell = quitBridgeMock();
    const { send, id } = await untitledProject(shell.bridge);
    await act(async () => {});
    shell.requestQuit();
    const dialog = await prompt();
    expect(screen.queryByRole("dialog", { name: "quit ethereal?" })).toBeNull();
    fireEvent.click(within(dialog).getByRole("button", { name: "discard & quit" }));
    await waitFor(() => expect(shell.bridge.quit).toHaveBeenCalledOnce());
    // The open project: the engine deletes it as it stops (`Project::Discard`).
    expect(send.mock.calls.map(([c]) => c)).toContainEqual({ domain: "Project", command: { type: "Discard", id } });
    expect(isUntitled(id)).toBe(false);
  });

  it("keep & quit quits without discarding; cancel stays", async () => {
    const shell = quitBridgeMock();
    const { send } = await untitledProject(shell.bridge);
    await act(async () => {});
    shell.requestQuit();
    fireEvent.click(within(await prompt()).getByRole("button", { name: "cancel" }));
    expect(shell.bridge.cancel).toHaveBeenCalledOnce();
    expect(shell.bridge.quit).not.toHaveBeenCalled();
    shell.requestQuit();
    fireEvent.click(within(await prompt()).getByRole("button", { name: "keep & quit" }));
    await waitFor(() => expect(shell.bridge.quit).toHaveBeenCalledOnce());
    expect(send.mock.calls.some(([c]) => c.domain === "Project" && c.command.type === "Discard")).toBe(false);
  });

  it("a named project gets the usual quit question", async () => {
    const shell = quitBridgeMock();
    const { mock, id } = await untitledProject(shell.bridge);
    await act(async () => void (await mock.send({ domain: "Project", command: { type: "Rename", id, name: "named" } })));
    await act(async () => {});
    shell.requestQuit();
    expect(await screen.findByRole("dialog", { name: "quit ethereal?" })).toBeInTheDocument();
  });
});

describe("Project::Discard (mock engine)", () => {
  it("the open project is never saved again and is deleted once another opens", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    await mock.connect();
    const id = mock.snapshot().id;
    const next = await other(mock, id);
    await mock.send({ domain: "Project", command: { type: "Discard", id } });
    await expect(mock.send({ domain: "Project", command: { type: "Save" } })).rejects.toThrow();
    expect(await stored(mock, id)).toBe(true);
    await mock.send({ domain: "Project", command: { type: "Open", id: next.id } });
    expect(await stored(mock, id)).toBe(false);
  });
});
