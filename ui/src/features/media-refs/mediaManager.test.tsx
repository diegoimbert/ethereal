import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, Device, MediaRef, Project, ReplyValue } from "@/generated";
import { ConfirmDialog } from "@/kit";
import { useProjectStore } from "@/state";
import type { EngineTransport } from "@/transport";
import { createDemoProject } from "@/transport/mock";
import { MediaManagerDialog } from "./MediaManagerDialog";
import { mediaRefCommands } from "./menus";
import { closeMediaManager, openMediaManager, resetMediaRefs } from "./store";
import { mediaUsage } from "./usage";

class FakeTransport {
  readonly kind = "mock" as const;
  sent: Command[] = [];
  connect = () => Promise.reject(new Error("unused"));
  send = async (c: Command): Promise<ReplyValue> => {
    this.sent.push(c);
    return { type: "Unit" };
  };
  onEvent = () => () => {};
  subscribePlayhead = () => () => {};
  subscribeMeters = () => () => {};
  dispose = () => {};
}

const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

let project: Project;
let used: MediaRef;
let unused: MediaRef;
let clipCount: number;

beforeEach(() => {
  const demo = createDemoProject();
  const audio = Object.values(demo.clips).find((c) => c.content.type === "Audio")!;
  const id = audio.content.type === "Audio" ? audio.content.media : "";
  used = { ...demo.media[id]!, name: "Kick.wav" };
  unused = { ...used, id: "01KUNUSED00000000000000000", name: "Unused.wav" };
  // A sampler on the first track loads the kick too.
  const track = Object.values(demo.tracks)[0]!;
  const sampler: Device = {
    ...Object.values(demo.devices)[0]!,
    id: "01KSAMPLER0000000000000000",
    track: track.id,
    kind: { type: "Builtin", device: { type: "Sampler", sample: id } } as Device["kind"],
  };
  project = { ...demo, media: { ...demo.media, [id]: used, [unused.id]: unused }, devices: { ...demo.devices, [sampler.id]: sampler } };
  clipCount = Object.values(project.clips).filter((c) => c.content.type === "Audio" && c.content.media === id).length;
  useProjectStore.setState({ project });
  resetMediaRefs();
});

afterEach(() => {
  closeMediaManager();
  useProjectStore.setState({ project: null });
});

function show(t: FakeTransport) {
  render(
    <>
      <MediaManagerDialog transport={t as unknown as EngineTransport} />
      <ConfirmDialog />
    </>,
  );
  act(() => openMediaManager());
}

const row = (m: MediaRef) => screen.getAllByTestId("media-manager-row").find((r) => r.dataset.media === m.id)!;

describe("media manager", () => {
  it("counts what uses each sample", () => {
    const u = mediaUsage(project);
    expect(u.get(used.id)!.clips.length).toBe(clipCount);
    expect(u.get(used.id)!.devices).toEqual(["01KSAMPLER0000000000000000"]);
    expect(u.get(unused.id)).toEqual({ clips: [], devices: [], freezes: [] });
  });

  it("lists every sample with its uses; removing one says what it unlinks first", async () => {
    const t = new FakeTransport();
    show(t);
    expect(within(row(used)).getByTestId("media-manager-usage").textContent).toBe(`${clipCount} clip${clipCount === 1 ? "" : "s"} · 1 sampler`);
    expect(within(row(unused)).getByTestId("media-manager-usage").textContent).toBe("unused");

    fireEvent.click(within(row(used)).getByTestId("media-manager-remove"));
    const confirm = screen.getByRole("dialog", { name: "remove this sample?" });
    expect(confirm.textContent).toContain("“Kick.wav” will be removed from the project.");
    expect(confirm.textContent).toContain(`${clipCount} audio clip`);
    expect(confirm.textContent).toContain("1 sampler is unlinked");
    expect(t.sent).toEqual([]);
    fireEvent.click(within(confirm).getByRole("button", { name: "remove" }));
    await flush();
    expect(t.sent).toEqual([{ domain: "MediaRef", command: { type: "Remove", media: [used.id] } }]);
  });

  it("removes every unused sample at once, and filters by name", async () => {
    const t = new FakeTransport();
    show(t);
    fireEvent.click(screen.getByTestId("media-manager-remove-unused"));
    const confirm = screen.getByRole("dialog", { name: "remove this sample?" });
    expect(confirm.textContent).not.toContain("clip");
    fireEvent.click(within(confirm).getByRole("button", { name: "remove" }));
    await flush();
    expect(t.sent).toEqual([{ domain: "MediaRef", command: { type: "Remove", media: [unused.id] } }]);

    fireEvent.change(screen.getByRole("searchbox", { name: "filter samples" }), { target: { value: "unus" } });
    expect(screen.getAllByTestId("media-manager-row").map((r) => r.dataset.media)).toEqual([unused.id]);
  });

  it("is in the command palette", () => {
    const entry = mediaRefCommands(new FakeTransport() as unknown as EngineTransport, project).find((c) => c.id === "media:manager");
    expect(entry?.label).toBe("media manager…");
  });
});
