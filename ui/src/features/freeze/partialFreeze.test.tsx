// Freeze up to a device (#25, partial chain freeze): the device-menu entry (never on MIDI
// effects), the frozen prefix of the chain, and the chain's frozen display.
import { act, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { BuiltinDevice, Command, Device, DeviceCategory, Project, ReplyValue, Track } from "@/generated";
import type { ContextMenuEntry, ContextMenuItem } from "@/kit";
import { devicesOfTrack, useProjectStore } from "@/state";
import { type EngineTransport } from "@/transport";
import { createDemoProject, type MockTransport } from "@/transport/mock";
import { renderWithMock, resetStores, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { DeviceChain } from "@/features/devices";
import { frozenChainLength, isDeviceFrozen, isFrozenTrack } from "./inert";
import { freezeDeviceEntries } from "./menus";
import { resetRenderJobs } from "./store";

class FakeTransport {
  readonly kind = "mock" as const;
  sent: Command[] = [];
  connect = () => Promise.reject(new Error("unused"));
  send = async (c: Command): Promise<ReplyValue> => {
    this.sent.push(c);
    return c.domain === "Freeze" && c.command.type === "Freeze" ? { type: "RenderStarted", job: c.command.job } : { type: "Unit" };
  };
  onEvent = () => () => {};
  subscribePlayhead = () => () => {};
  subscribeMeters = () => () => {};
  dispose = () => {};
}

const labels = (entries: ContextMenuEntry[]) => entries.map((e) => (e === "separator" ? "—" : e.label));
const CATEGORY: Partial<Record<BuiltinDevice["type"], DeviceCategory>> = { Arpeggiator: "NoteEffect", Synth: "Instrument" };
const categoryOf = (d: Device): DeviceCategory => (d.kind.type === "Builtin" ? (CATEGORY[d.kind.device.type] ?? "AudioEffect") : "AudioEffect");

let project: Project;
let fake: FakeTransport;
let transport: EngineTransport;
let track: Track;
let chain: Device[];

/** A MIDI track with clips and the chain arpeggiator → synth → delay → utility. */
function setup() {
  project = createDemoProject();
  track = Object.values(project.tracks).find((t) => t.kind === "Midi" && Object.values(project.clips).some((c) => c.track === t.id))!;
  const devices = { ...project.devices };
  for (const d of Object.values(devices)) if (d.track === track.id) delete devices[d.id];
  const kinds: BuiltinDevice[] = [{ type: "Arpeggiator" }, { type: "Synth" }, { type: "Delay" }, { type: "Utility" }];
  chain = kinds.map((device, i) => ({
    id: `01J0000000000000000000000${i}`,
    track: track.id,
    order: `a${i}`,
    name: device.type.toLowerCase(),
    enabled: true,
    kind: { type: "Builtin", device },
    params: {},
    sidechain: null,
    pad: null,
  }));
  for (const d of chain) devices[d.id] = d;
  project = { ...project, devices };
  useProjectStore.setState({ project });
}

function freezeAt(upTo: Device | null) {
  const t = { ...track, freeze: { media: "m", start: 0, ...(upTo ? { up_to: upTo.id } : {}) } };
  project = { ...project, tracks: { ...project.tracks, [t.id]: t } };
  useProjectStore.setState({ project });
  track = t;
}

beforeEach(() => {
  resetRenderJobs();
  fake = new FakeTransport();
  transport = fake as unknown as EngineTransport;
  setup();
});

afterEach(() => {
  useProjectStore.setState({ project: null });
});

describe("freeze up to here", () => {
  it("is offered from the instrument on, never on MIDI effects or devices inside racks", () => {
    const [arp, synth, delay, utility] = chain as [Device, Device, Device, Device];
    expect(freezeDeviceEntries(transport, arp, categoryOf)).toEqual([]);
    for (const d of [synth, delay, utility]) expect(labels(freezeDeviceEntries(transport, d, categoryOf))).toEqual(["freeze up to here"]);
    // A device on a rack chain or a drum pad: not a freeze point.
    expect(freezeDeviceEntries(transport, { ...delay, chain: "01J00000000000000000000099" }, categoryOf)).toEqual([]);
    // A return track can't be frozen.
    const ret = Object.values(project.tracks).find((t) => t.kind === "Return");
    if (ret) expect(freezeDeviceEntries(transport, { ...delay, track: ret.id }, categoryOf)).toEqual([]);
  });

  it("sends a freeze with the device as freeze point", () => {
    const delay = chain[2]!;
    const entry = freezeDeviceEntries(transport, delay, categoryOf)[0] as ContextMenuItem;
    entry.onSelect();
    return new Promise<void>((resolve) =>
      setTimeout(() => {
        const sent = fake.sent.find((c) => c.domain === "Freeze");
        expect(sent).toMatchObject({ domain: "Freeze", command: { type: "Freeze", track: track.id, up_to: delay.id } });
        resolve();
      }, 0),
    );
  });

  it("freezes the freeze point and every device before it; the rest stays live", () => {
    const [arp, synth, delay, utility] = chain as [Device, Device, Device, Device];
    freezeAt(synth);
    expect(isFrozenTrack(project, track)).toBe(true);
    expect(frozenChainLength(project, track.id)).toBe(2);
    expect([arp, synth, delay, utility].map((d) => isDeviceFrozen(project, d))).toEqual([true, true, false, false]);
    // The live tail offers nothing; a frozen device offers to unfreeze.
    expect(freezeDeviceEntries(transport, delay, categoryOf)).toEqual([]);
    expect(labels(freezeDeviceEntries(transport, synth, categoryOf))).toEqual(["unfreeze track"]);
    // A plain freeze (no freeze point, as in older projects) freezes the whole chain.
    freezeAt(null);
    expect(frozenChainLength(project, track.id)).toBe(4);
    expect(chain.every((d) => isDeviceFrozen(project, d))).toBe(true);
  });
});

describe("frozen chain display", () => {
  let mock: MockTransport | undefined;
  beforeAll(stubPointerCapture);
  afterEach(() => {
    resetStores(mock);
    mock = undefined;
  });

  it("styles the frozen prefix of the chain as frozen and leaves the live tail editable", async () => {
    mock = await renderWithMock(<DeviceChain />);
    await screen.findByRole("region", { name: "Synth" });
    const keys = trackByName("Keys");
    const p = useProjectStore.getState().project!;
    const devices = devicesOfTrack(p, keys.id);
    expect(devices.length).toBeGreaterThan(1);
    const [first, ...rest] = devices as [Device, ...Device[]];
    act(() =>
      useProjectStore.setState({
        project: { ...p, tracks: { ...p.tracks, [keys.id]: { ...keys, freeze: { media: "m", start: 0, up_to: first.id } } } },
      }),
    );
    const region = (d: Device) => screen.getByRole("region", { name: d.name });
    expect(region(first).dataset.frozen).toBe("true");
    expect(region(first).classList.contains("eth-device--frozen")).toBe(true);
    expect(screen.getAllByRole("img", { name: "frozen" }).length).toBe(1);
    for (const d of rest) {
      expect(region(d).dataset.frozen).toBeUndefined();
      expect(region(d).classList.contains("eth-device--frozen")).toBe(false);
    }
  });
});
