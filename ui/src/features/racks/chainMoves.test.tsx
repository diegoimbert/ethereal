/** Rack chain moves (MockTransport): drag a chain row to reorder it, onto another rack (cmd/ctrl
 * copies), refused drops, and the row's copy / cut / paste / duplicate menu. */
import { act, cleanup, createEvent, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { BuiltinDevice, DeviceId, RackChainId } from "@/generated";
import { DeviceChain } from "@/features/devices";
import { flush, renderWithMock, resetStores, stubPointerCapture, store, trackByName } from "@/features/devices/testUtils";
import { useContextMenuStore } from "@/kit";
import { cmd, newId, type MockTransport } from "@/transport";
import { chainsOf, dropIndex, useChainClipboard, useChainDrag } from "./chainClipboard";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  cleanup();
  resetStores(mock);
  mock = undefined;
  useChainClipboard.setState({ selected: null, copied: null });
  useChainDrag.setState({ dragging: null, over: null });
});

const send = async (c: Parameters<MockTransport["send"]>[0]) => {
  await act(() => mock!.send(c));
  await flush();
};

async function insert(type: string, name: string): Promise<DeviceId> {
  const id = newId();
  await send(
    cmd("Device", { type: "Insert", id, track: trackByName("Keys").id, device: { type: "Builtin", device: { type } as BuiltinDevice }, before: null }),
  );
  await send(cmd("Device", { type: "Rename", id, name }));
  return id;
}

/** An audio effect rack named `name` with chains named `chains` (its first chain renamed). */
async function rackWith(name: string, chains: string[]): Promise<DeviceId> {
  const rack = await insert("AudioEffectRack", name);
  const first = chainsOf(store().project!, rack)[0]!.id;
  await send(cmd("Rack", { type: "RenameChain", id: first, name: chains[0]! }));
  for (const n of chains.slice(1)) await send(cmd("Rack", { type: "AddChain", id: newId(), rack, name: n, before: null }));
  return rack;
}

const names = (rack: DeviceId) => chainsOf(store().project!, rack).map((c) => c.name);
const region = (name: string) => screen.getByRole("region", { name });
const row = (rack: string, chain: string) => within(region(rack)).getByRole("button", { name: `${chain} chain` });
const list = (rack: string) => region(rack).querySelector<HTMLElement>(".eth-rack__chains")!;

/** A DataTransfer stand-in (jsdom has none). */
function dataTransfer() {
  const data: Record<string, string> = {};
  return {
    types: [] as string[],
    dropEffect: "none",
    effectAllowed: "all",
    setData(type: string, value: string) {
      this.types.push(type);
      data[type] = value;
    },
    getData: (type: string) => data[type] ?? "",
  };
}

/** Drags `from` (a chain row) over `to` (a chain list) and drops it; jsdom's rows have no size:
 *  `clientY` < 0 lands before the first chain, > 0 after the last. */
async function drag(from: HTMLElement, to: HTMLElement, clientY: number, mods: { ctrlKey?: boolean } = {}) {
  const dt = dataTransfer();
  fireEvent.pointerDown(from.querySelector(".eth-rack__name")!, { button: 0 });
  fireEvent.dragStart(from, { dataTransfer: dt });
  // jsdom has no DragEvent: the pointer position and keys are set on the plain event.
  const at = (ev: Event) => {
    Object.defineProperties(ev, { clientY: { value: clientY }, ctrlKey: { value: !!mods.ctrlKey }, metaKey: { value: false } });
    return ev;
  };
  fireEvent(to, at(createEvent.dragOver(to, { dataTransfer: dt })));
  const effect = dt.dropEffect;
  const line = screen.queryAllByTestId("chain-insert").length;
  fireEvent(to, at(createEvent.drop(to, { dataTransfer: dt })));
  fireEvent.dragEnd(from, { dataTransfer: dt });
  await flush();
  return { effect, line };
}

function menuItem(label: string) {
  const item = useContextMenuStore.getState().menu?.items.find((i) => i !== "separator" && i.label === label);
  if (!item || item === "separator") throw new Error(`no ${label} entry`);
  return item;
}

async function render() {
  mock = await renderWithMock(<DeviceChain />);
  await screen.findByRole("region", { name: "Synth" });
}

describe("rack chain moves", () => {
  it("drop line index: before the first row whose middle is below the pointer", () => {
    const rows = [
      { top: 0, height: 20 },
      { top: 24, height: 20 },
    ];
    expect(dropIndex(rows, 5)).toBe(0);
    expect(dropIndex(rows, 15)).toBe(1);
    expect(dropIndex(rows, 40)).toBe(2);
  });

  it("drags the third chain first (one undo step), and onto another rack (cmd/ctrl: a copy)", async () => {
    await render();
    const a = await rackWith("rack a", ["one", "two", "three"]);
    const b = await rackWith("rack b", ["four"]);
    const first = await drag(row("rack a", "three"), list("rack a"), -1);
    expect(first.effect).toBe("move");
    expect(first.line).toBe(1);
    expect(names(a)).toEqual(["three", "one", "two"]);
    // Kept its id (a reorder).
    expect(chainsOf(store().project!, a)[0]!.name).toBe("three");
    await send(cmd("Edit", { type: "Undo" }));
    expect(names(a)).toEqual(["one", "two", "three"]);

    // Onto rack b, at its end: moved.
    await drag(row("rack a", "two"), list("rack b"), 1);
    expect(names(a)).toEqual(["one", "three"]);
    expect(names(b)).toEqual(["four", "two"]);
    // Ctrl held: copied, the source stays.
    const copy = await drag(row("rack a", "one"), list("rack b"), -1, { ctrlKey: true });
    expect(copy.effect).toBe("copy");
    expect(names(a)).toEqual(["one", "three"]);
    expect(names(b)).toEqual(["one", "four", "two"]);
  });

  it("refuses a chain where it can't go: not-allowed cursor, no line, nothing changes", async () => {
    await render();
    const outer = await rackWith("outer", ["x", "y"]);
    // A rack nested on chain "x": "x" can't go inside it.
    const x: RackChainId = chainsOf(store().project!, outer)[0]!.id;
    const nested = newId();
    await send(
      cmd("Rack", {
        type: "InsertDevice",
        id: nested,
        chain: x,
        device: { type: "Builtin", device: { type: "AudioEffectRack" } as BuiltinDevice },
        before: null,
      }),
    );
    await send(cmd("Device", { type: "Rename", id: nested, name: "inner" }));
    fireEvent.click(row("outer", "x"));
    await flush();
    const before = store().project!.rack_chains;
    const refused = await drag(row("outer", "x"), list("inner"), 1);
    expect(refused.effect).toBe("none");
    expect(refused.line).toBe(0);
    expect(store().project!.rack_chains).toBe(before);
    // Its sibling may go in.
    await drag(row("outer", "y"), list("inner"), 1);
    expect(names(nested)).toEqual(["chain 1", "y"]);
  });

  it("copies, cuts, pastes and duplicates from a chain's context menu", async () => {
    await render();
    const a = await rackWith("rack a", ["one", "two"]);
    const b = await rackWith("rack b", ["three"]);
    fireEvent.contextMenu(row("rack a", "one"));
    // Nothing copied yet: paste is off.
    expect(menuItem("paste after").disabled).toBe(true);
    act(() => menuItem("copy").onSelect());
    fireEvent.contextMenu(row("rack b", "three"));
    await act(async () => menuItem("paste after").onSelect());
    await flush();
    expect(names(b)).toEqual(["three", "one"]);
    expect(names(a)).toEqual(["one", "two"]);
    // Cut "two", paste it in rack b after "three": moved, and the clipboard empties.
    fireEvent.contextMenu(row("rack a", "two"));
    act(() => menuItem("cut").onSelect());
    fireEvent.contextMenu(row("rack b", "three"));
    await act(async () => menuItem("paste after").onSelect());
    await flush();
    expect(names(a)).toEqual(["one"]);
    expect(names(b)).toEqual(["three", "two", "one"]);
    expect(useChainClipboard.getState().copied).toBeNull();
    // Duplicate: right after itself.
    fireEvent.contextMenu(row("rack a", "one"));
    await act(async () => menuItem("duplicate").onSelect());
    await flush();
    expect(names(a)).toEqual(["one", "one"]);
    // Rename and delete are still there.
    fireEvent.contextMenu(within(region("rack a")).getAllByRole("button", { name: "one chain" })[0]!);
    expect(menuItem("rename chain")).toBeDefined();
    expect(menuItem("delete chain").danger).toBe(true);
  });
});
