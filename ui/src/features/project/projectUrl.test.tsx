// The open project in the address bar: `/p/<slug>/<id>` (slug, parsing, launch from the URL,
// rename, switching projects, Back / Forward, other routes left alone).
import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Project } from "@/generated";
import { resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { TransportProvider, type EngineTransport } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { ProjectMenu } from "./index";
import { useLaunchPrefs } from "./launch";
import { useProjectScreen } from "./screenStore";
import { resetDrafts } from "./drafts";
import { MISSING_PROJECT_NOTICE, parseProjectPath, projectPath, projectSlug, urlSyncEnabled } from "./projectUrl";
import { t } from "@/i18n";

describe("projectSlug", () => {
  it.each([
    ["My Song", "my-song"],
    ["Café Déjà Vu", "cafe-deja-vu"],
    ["Straße Øresund Æther", "strasse-oresund-aether"],
    ["  --Hello,,,   World!!  ", "hello-world"],
    ["Beat 🎵 #2", "beat-2"],
    ["🎵🎶", "untitled"],
    ["夜の曲", "untitled"],
    ["夜 Night 曲", "night"],
    ["", "untitled"],
    ["   ", "untitled"],
    ["10/12/2026 project", "10-12-2026-project"],
  ])("%j → %s", (name, slug) => expect(projectSlug(name)).toBe(slug));

  it("is capped at 60 characters, without a trailing dash", () => {
    const slug = projectSlug(`${"a".repeat(59)} b and more`);
    expect(slug).toBe("a".repeat(59));
    expect(projectSlug("x".repeat(200))).toHaveLength(60);
  });
});

describe("project paths", () => {
  it("formats and parses /p/<slug>/<id>", () => {
    const id = "01890a5d-ac96-774b-bcce-b302099a8057";
    expect(projectPath("Café", id)).toBe(`/p/cafe/${id}`);
    expect(parseProjectPath(`/p/cafe/${id}`)).toEqual({ slug: "cafe", id });
    expect(parseProjectPath(`/p/cafe/${id}/`)).toEqual({ slug: "cafe", id });
    // Missing slug: the id alone still works.
    expect(parseProjectPath(`/p/${id}`)).toEqual({ slug: null, id });
    expect(parseProjectPath(`/p//${id}`)).toEqual({ slug: "", id });
  });

  it("is null for other routes", () => {
    for (const p of ["/", "/p", "/p/", "/s/abcdefghijklmnopqrstuv", "/join/room", "/p/a/b/c", "/x/a/b"]) expect(parseProjectPath(p)).toBeNull();
  });

  it("is off on the desktop app", () => {
    expect(urlSyncEnabled({ kind: "tauri" })).toBe(false);
    expect(urlSyncEnabled({ kind: "wasm" })).toBe(true);
    expect(urlSyncEnabled(null)).toBe(false);
  });
});

/** A host that connects with nothing open, like every real host (the mock opens its demo). */
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

const store = () => useProjectStore.getState();

async function storedProjects(mock: MockTransport) {
  const reply = await mock.send({ domain: "Project", command: { type: "List" } });
  if (reply.type !== "Projects") throw new Error("no list");
  return reply.projects;
}

function mount(mock: MockTransport) {
  render(
    <TransportProvider transport={new NothingOpen(mock)}>
      <ProjectMenu />
    </TransportProvider>,
  );
}

const open = (mock: MockTransport, id: string) => act(async () => void (await mock.send({ domain: "Project", command: { type: "Open", id } })));

describe("the address bar follows the open project", () => {
  beforeEach(() => {
    localStorage.clear();
    useLaunchPrefs.setState({ reopenLast: false });
    useProjectScreen.setState({ launching: true });
  });
  afterEach(() => {
    resetDrafts();
    resetStores();
    useProjectScreen.setState({ open: false, launchPending: false, launching: false, notice: null });
    localStorage.clear();
    history.replaceState(null, "", "/");
  });

  it("launch opens the URL's project and replaces a stale slug (query kept)", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    const target = (await storedProjects(mock)).find((p) => p.id !== mock.snapshot().id)!;
    history.replaceState(null, "", `/p/old-name/${target.id}?force-desktop`);
    const before = history.length;
    mount(mock);
    await waitFor(() => expect(store().project?.id).toBe(target.id));
    await waitFor(() => expect(location.pathname).toBe(projectPath(target.name, target.id)));
    expect(location.search).toBe("?force-desktop");
    expect(history.length).toBe(before);
    expect(useProjectScreen.getState().open).toBe(false);
  });

  it("an unknown id shows the picker with a notice and the URL becomes /", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    history.replaceState(null, "", "/p/gone/00000000-0000-7000-8000-000000000000");
    mount(mock);
    await screen.findByRole("dialog", { name: "open project" });
    expect(await screen.findByTestId("project-screen-notice")).toHaveTextContent(t(MISSING_PROJECT_NOTICE));
    await waitFor(() => expect(location.pathname).toBe("/"));
  });

  it("the launch draft behind the picker stays at /; rename replaces, switching pushes, Back opens", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    mount(mock);
    await screen.findByRole("dialog", { name: "open project" });
    await waitFor(() => expect(store().project).not.toBeNull());
    expect(location.pathname).toBe("/");

    const [a, b] = (await storedProjects(mock)).filter((p) => p.id !== store().project!.id);
    // From /: replaced (no entry for the picker).
    const start = history.length;
    await open(mock, a!.id);
    act(() => useProjectScreen.getState().hide());
    await waitFor(() => expect(location.pathname).toBe(projectPath(a!.name, a!.id)));
    expect(history.length).toBe(start);

    // Rename: same entry, new slug.
    await act(async () => void (await mock.send({ domain: "Project", command: { type: "Rename", id: a!.id, name: "Été 2026!" } })));
    await waitFor(() => expect(location.pathname).toBe(`/p/ete-2026/${a!.id}`));
    expect(history.length).toBe(start);

    // Another project: a new entry.
    await open(mock, b!.id);
    await waitFor(() => expect(location.pathname).toBe(projectPath(b!.name, b!.id)));
    expect(history.length).toBe(start + 1);

    // Back: the URL's project opens (simulated: jsdom's history.back is async and partial).
    act(() => {
      history.replaceState(null, "", `/p/ete-2026/${a!.id}`);
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    await waitFor(() => expect(store().project?.id).toBe(a!.id));
    expect(location.pathname).toBe(`/p/ete-2026/${a!.id}`);
  });

  it("opening the picker over a stored project keeps its URL", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    mount(mock);
    await screen.findByRole("dialog", { name: "open project" });
    const a = (await storedProjects(mock)).find((p) => p.id !== store().project?.id)!;
    await open(mock, a.id);
    act(() => useProjectScreen.getState().hide());
    await waitFor(() => expect(location.pathname).toBe(projectPath(a.name, a.id)));
    act(() => useProjectScreen.getState().show());
    expect(location.pathname).toBe(projectPath(a.name, a.id));
  });
});
