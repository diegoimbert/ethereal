// The Pitch MIDI effect's declared layout (mock descriptor = the Rust one, kept equal by
// `v02_descriptors.rs`) as the renderer resolves and draws it: the transpose knob, the range
// knobs and the out-of-range mode, each reaching the device's params.
import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device } from "@/generated";
import { useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, cmd, newId } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { formatParam } from "@/domain/paramScale";
import { useGestureSender } from "../gesture";
import { deviceWindowWidth } from "../window/sizes";
import { DeviceLayoutView } from "./DeviceLayoutView";
import { boundParams, resolveLayout } from "./model";

const PITCH = BUILTIN_DESCRIPTORS.Pitch;
let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

function Panel({ id }: { id: string }) {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[id]);
  if (!device) return null;
  return (
    <section aria-label="panel">
      <DeviceLayoutView device={device} descriptor={PITCH} sender={sender} />
    </section>
  );
}

describe("Pitch layout", () => {
  it("binds every param in two sections, nothing under More", () => {
    const name = new Map(PITCH.params.map((p) => [p.id, p.name]));
    const outline = PITCH.layout!.sections.map((s) => `${s.id}: ${s.items.flatMap((i) => boundParams(i.widget).map((p) => name.get(p))).join(", ")}`);
    expect(outline).toEqual(["pitch: Pitch", "range: Lowest, Range, Mode"]);
    const r = resolveLayout(PITCH);
    expect(r.declared).toBe(true);
    expect(r.more).toBeNull();
    expect(PITCH.category).toBe("NoteEffect");
    // A small device: the base window width.
    const width = (type: "Pitch" | "Velocity") => deviceWindowWidth({ kind: { type: "Builtin", device: { type } } } as unknown as Pick<Device, "kind">);
    expect(width("Pitch")).toBe(width("Velocity"));
  });

  it("shows Live's ranges: ±128 semitones, a 0..127 note range", () => {
    const [pitch, lowest, range, mode] = PITCH.params;
    expect([pitch!.min, pitch!.max, pitch!.default, pitch!.step]).toEqual([-128, 128, 0, 1]);
    expect(pitch!.unit).toBe("Semitones");
    expect(formatParam(pitch!, -12)).toBe("-12");
    expect([lowest!.min, lowest!.max, lowest!.default]).toEqual([0, 127, 0]);
    expect([range!.min, range!.max, range!.default]).toEqual([0, 127, 127]);
    expect(mode!.labels).toEqual(["Block", "Fold", "Limit"]);
  });

  it("renders its controls and sets the device's params", async () => {
    const id = newId();
    mock = await renderWithMock(<Panel id={id} />);
    const keys = trackByName("Keys").id;
    const synth = Object.values(store().project!.devices).find((d) => d.track === keys && d.name === "Synth")!.id;
    await act(async () => {
      await mock!.send(cmd("Device", { type: "Insert", id, track: keys, device: { type: "Builtin", device: { type: "Pitch" } }, before: synth }));
    });
    await flush();
    const panel = screen.getByRole("region", { name: "panel" });
    for (const name of ["Pitch", "Lowest", "Range"]) expect(within(panel).getByRole("slider", { name })).toBeTruthy();
    expect(panel.querySelector(".eth-widget--missing")).toBeNull();
    const modes = within(panel).getByRole("group", { name: "Mode" });
    fireEvent.click(within(modes).getByRole("button", { name: "Fold" }));
    await flush();
    expect(store().project!.devices[id]!.params[3]).toBe(1);
  });
});
