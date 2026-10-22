// Launch against a desktop-like engine: commands are handled one at a time and the project list
// is slow (a big library), so every extra `Project::List` delays everything queued behind it.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { Project } from "@/generated";
import { resetStores } from "@/features/transport-bar/testUtils";
import { useProjectScreen } from "@/features/project/screenStore";
import { resetDrafts } from "@/features/project/drafts";
import { TransportProvider, type EngineTransport } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { App } from "./App";

const SLOW = ["Project.List", "Version.ListRecoverable", "Version.SessionStatus", "Template.NewProject"];

class SerialEngine implements EngineTransport {
  readonly kind = "tauri" as const;
  readonly handled: string[] = [];
  private queue: Promise<unknown> = Promise.resolve();
  constructor(readonly inner: MockTransport) {}
  async connect(): Promise<Project | null> {
    await this.inner.connect();
    return null;
  }
  send: EngineTransport["send"] = (c, o) => {
    const name = `${c.domain}.${(c.command as { type: string }).type}`;
    const reply = this.queue.then(async () => {
      this.handled.push(name);
      await new Promise((r) => setTimeout(r, SLOW.includes(name) ? 150 : 2));
      return this.inner.send(c, o);
    });
    this.queue = reply.catch(() => undefined);
    return reply;
  };
  onEvent: EngineTransport["onEvent"] = (l) => this.inner.onEvent(l);
  subscribePlayhead: EngineTransport["subscribePlayhead"] = (l) => this.inner.subscribePlayhead(l);
  subscribeMeters: EngineTransport["subscribeMeters"] = (l) => this.inner.subscribeMeters(l);
  dispose() {
    this.inner.dispose();
  }
}

afterEach(() => {
  cleanup();
  resetDrafts();
  resetStores();
  useProjectScreen.setState({ open: false, launchPending: false, launching: false });
});

it("launch lists the projects at most twice, then not again while idle", async () => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  const engine = new SerialEngine(new MockTransport({ timers: "manual", seed: 7 }));
  render(
    <TransportProvider transport={engine}>
      <App />
    </TransportProvider>,
  );
  const lists = () => engine.handled.filter((n) => n === "Project.List").length;
  await act(() => new Promise((r) => setTimeout(r, 2000)));
  expect(useProjectScreen.getState().open).toBe(true);
  expect(lists()).toBeGreaterThanOrEqual(1);
  expect(lists()).toBeLessThanOrEqual(2);
  const settled = engine.handled.length;
  await act(() => new Promise((r) => setTimeout(r, 1000)));
  expect(engine.handled.slice(settled).filter((n) => n === "Project.List")).toEqual([]);
}, 15000);
