import { act, createEvent, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { DeviceId, MediaId, SampleZone } from "@/generated";
import { BROWSER_DRAG_MIME } from "@/features/browser/dragPayload";
import { useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, cmd, newId, type MockTransport } from "@/transport";
import { LIBRARY_ID } from "@/transport/mock/library";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/mixer/testUtils";
import { useGestureSender } from "../../gesture";
import { DeviceLayoutView } from "../DeviceLayoutView";
import { newZone } from "./zoneMath";

const DESC = BUILTIN_DESCRIPTORS.MultiSampler;
let mock: MockTransport | undefined;
let msId: DeviceId = "";
// jsdom has no layout: every box is 200 × 72 (128 keys → 1.5625 px per key).
const rect = { left: 0, top: 0, right: 200, bottom: 72, width: 200, height: 72, x: 0, y: 0, toJSON: () => ({}) };
beforeAll(() => {
  stubPointerCapture();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(rect as DOMRect);
  (SVGElement.prototype as unknown as { setPointerCapture: () => void }).setPointerCapture ??= () => {};
});
afterAll(() => vi.restoreAllMocks());
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

const zones = (): SampleZone[] => {
  const k = store().project!.devices[msId]!.kind;
  return k.type === "Builtin" && k.device.type === "MultiSampler" ? k.device.zones : [];
};

function Panel() {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[msId]);
  if (!device) return null;
  return (
    <section aria-label="panel">
      <DeviceLayoutView device={device} descriptor={DESC} sender={sender} />
    </section>
  );
}

async function importLib(path: string): Promise<MediaId> {
  const reply = await mock!.send(cmd("Media", { type: "Import", id: newId(), source: { type: "Location", location: { type: "Library", id: LIBRARY_ID }, path } }));
  if (reply.type !== "Media") throw new Error(reply.type);
  return reply.media.id;
}

async function renderMap() {
  mock = await renderWithMock(<Panel />);
  msId = newId();
  await act(async () => {
    await mock!.send(
      cmd("Device", { type: "Insert", id: msId, track: trackByName("Keys").id, device: { type: "Builtin", device: { type: "MultiSampler", zones: [] } }, before: null }),
    );
    const a = await importLib("Synths/Pad C.wav");
    const b = await importLib("Synths/Bass A1.wav");
    await mock!.send(cmd("Device", { type: "SetZones", device: msId, zones: [newZone(b, 45, { lo: 0, hi: 63 }), newZone(a, 72, { lo: 64, hi: 127 })] }));
  });
  await flush();
  const panel = screen.getByRole("region", { name: "panel" });
  const svg = within(panel).getByTestId("widget-zone-map").querySelector("svg")!;
  return { panel, svg, map: within(panel).getByTestId("zone-map") };
}

async function press(svg: SVGSVGElement, from: { x: number; y: number }, to = from) {
  await act(async () => {
    fireEvent.pointerDown(svg, { button: 0, pointerId: 1, clientX: from.x, clientY: from.y });
  });
  await act(async () => {
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: to.x, clientY: to.y });
  });
  await act(async () => {
    fireEvent.pointerUp(svg, { pointerId: 1, clientX: to.x, clientY: to.y });
  });
  await flush();
}

describe("ZoneMap", () => {
  it("draws the zones and selects one to inspect it", async () => {
    const { svg, panel } = await renderMap();
    expect(svg.querySelectorAll("[data-zone]")).toHaveLength(2);
    expect(within(panel).queryByTestId("zone-inspector")).toBeNull();
    await press(svg, { x: 150, y: 36 });
    const inspector = within(panel).getByTestId("zone-inspector");
    expect(inspector.textContent).toContain("Zone 2");
    expect(inspector.textContent).toContain("Pad C.wav");
    expect(svg.querySelector(".eth-zones__zone--selected")?.getAttribute("data-zone")).toBe("1");
    // A click on empty space (none here: full map) / Escape deselects.
    fireEvent.keyDown(within(panel).getByTestId("zone-map"), { key: "Escape" });
    await flush();
    expect(within(panel).queryByTestId("zone-inspector")).toBeNull();
  });

  it("resizing a zone's edge is one SetZones (one undo step)", async () => {
    const { svg } = await renderMap();
    await press(svg, { x: 150, y: 36 });
    const sent = vi.spyOn(mock!, "send");
    // Left edge of zone 2 is at x = 100: drag it 50 px left (32 keys).
    await press(svg, { x: 100, y: 36 }, { x: 50, y: 36 });
    expect(zones()[1]!.keys).toEqual({ lo: 32, hi: 127 });
    expect(sent.mock.calls.filter(([c]) => c.domain === "Device" && c.command.type === "SetZones")).toHaveLength(1);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(zones()[1]!.keys).toEqual({ lo: 64, hi: 127 });
  });

  it("edits the selected zone in the inspector and deletes it with Delete", async () => {
    const { svg, panel, map } = await renderMap();
    await press(svg, { x: 20, y: 36 });
    const root = within(panel).getByRole("spinbutton", { name: "Root" }) as HTMLInputElement;
    await act(async () => {
      fireEvent.focus(root);
      fireEvent.change(root, { target: { value: "48" } });
      fireEvent.keyDown(root, { key: "Enter" });
    });
    await flush();
    expect(zones()[0]!.root_key).toBe(48);
    await act(async () => {
      fireEvent.click(within(panel).getByRole("switch", { name: "Loop" }));
    });
    await flush();
    expect(zones()[0]!.looping).toBe(true);
    expect(zones()[0]!.loop_end).toBeGreaterThan(0);
    fireEvent.keyDown(map, { key: "Delete" });
    await flush();
    expect(zones()).toHaveLength(1);
    expect(zones()[0]!.keys).toEqual({ lo: 64, hi: 127 });
  });

  it("a Browser sample dropped on a key becomes a zone (import + zone = one undo step)", async () => {
    const { map } = await renderMap();
    const payload = {
      version: 1,
      kind: "media",
      source: { type: "Location", location: { type: "Library", id: LIBRARY_ID }, path: "Drums/Kick.wav" },
      name: "Kick.wav",
      file_kind: "Audio",
    };
    const dataTransfer = { types: [BROWSER_DRAG_MIME], getData: (t: string) => (t === BROWSER_DRAG_MIME ? JSON.stringify(payload) : ""), dropEffect: "none" };
    const media = Object.keys(store().project!.media).length;
    await act(async () => {
      // jsdom drag events carry no coordinates: set clientX by hand.
      for (const make of [createEvent.dragOver, createEvent.drop]) {
        const ev = make(map, { dataTransfer });
        Object.defineProperty(ev, "clientX", { value: 150 });
        fireEvent(map, ev);
      }
    });
    await flush();
    await flush();
    expect(screen.queryByRole("alert")?.textContent ?? null).toBeNull();
    expect(zones()).toHaveLength(3);
    // No note in the name: rooted at the drop key (x 150 → key 96), one key wide.
    expect(zones()[2]).toMatchObject({ root_key: 96, keys: { lo: 96, hi: 96 } });
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(zones()).toHaveLength(2);
    expect(Object.keys(store().project!.media)).toHaveLength(media);
  });
});
