// base-132: a plugin with 10,000 params never renders all of them. A generic layout card
// shows at most PARAM_CARD_CAP controls (pins → the plugin's quick controls → first
// automatable params); "Show all N parameters…" opens a searchable list that mounts only the
// rows in view; pins persist per plugin in UI settings.
// base-153: in the device chain, a plugin's card is its header; its "Parameters" toggle
// shows the same windowed, searchable list inline (capped height).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { Device, DeviceDescriptor } from "@/generated";
import { ContextMenuHost, VIRTUAL_MAX_ROWS } from "@/kit";
import { targetKey } from "@/features/automation/params";
import { useAutomationUi } from "@/features/automation/uiStore";
import { useProjectStore } from "@/state";
import { cmd, MOCK_MEGA_PARAM_COUNT, MOCK_MEGA_PLUGIN_ID, MockTransport, newId, TransportProvider } from "@/transport";
import { mockPluginDescriptor, MOCK_MEGA_REMOTE_FIRST } from "@/transport/mock/plugins";
import { flush, renderWithMock, resetStores, store, stubPointerCapture } from "@/features/devices/testUtils";
import { DeviceChain } from "../index";
import { useGestureSender } from "../gesture";
import { DeviceLayoutView } from "../layout";
import { cardParams, PARAM_CARD_CAP, resolveLayout } from "../layout/model";
import { getPins, pinKey, reloadPins, resetPins, setPinned } from "./pins";

const MEGA = mockPluginDescriptor(MOCK_MEGA_PLUGIN_ID)!;
const VISIBLE = MEGA.params.filter((p) => !p.hidden).length;
const KEY = `Clap:${MOCK_MEGA_PLUGIN_ID}`;

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
beforeEach(() => resetPins());
afterEach(() => {
  cleanup();
  resetStores(mock);
  mock = undefined;
  resetPins();
});

/** The first track of the demo project (the chain `<DeviceChain />` shows). */
const firstTrack = () => Object.values(store().project!.tracks).find((t) => t.kind !== "Master")!;

async function insertMega(m: MockTransport): Promise<Device> {
  const id = newId();
  await act(async () => {
    await m.send(
      cmd("Device", { type: "Insert", id, track: firstTrack().id, device: { type: "Plugin", plugin_id: MOCK_MEGA_PLUGIN_ID, sandboxed: null }, before: null }),
    );
  });
  await flush();
  return store().project!.devices[id]!;
}

async function renderMega(): Promise<HTMLElement> {
  mock = await renderWithMock(<DeviceChain />);
  await insertMega(mock);
  return screen.findByRole("region", { name: "Mock Mega" }, { timeout: 5000 });
}

/** Shows the card's inline parameter list (the header's "Parameters" toggle). */
async function showParams(card: HTMLElement): Promise<HTMLElement> {
  fireEvent.click(within(card).getByRole("button", { name: "show Mock Mega parameters" }));
  return within(card).findByRole("list", { name: "Mock Mega parameters" }, { timeout: 5000 });
}

/** A generic-layout card of the Mock Mega plugin (the shared renderer), outside the chain. */
function Card({ device, descriptor }: { device: Device; descriptor: DeviceDescriptor }) {
  const sender = useGestureSender();
  return (
    <section aria-label="perf card">
      <DeviceLayoutView device={device} descriptor={descriptor} sender={sender} />
    </section>
  );
}
const megaDevice = (id: string): Device => ({
  id,
  track: "t",
  order: "a",
  name: "Mock Mega",
  enabled: true,
  kind: {
    type: "Plugin",
    plugin: { format: "Clap", plugin_id: MOCK_MEGA_PLUGIN_ID, name: "Mock Mega", vendor: "", version: "", sandboxed: false, state: null },
  },
  params: {},
  sidechain: null,
  pad: null,
});

const cardParamIds = (card: HTMLElement) => [...card.querySelectorAll<HTMLElement>("[data-param]")].map((e) => Number(e.dataset.param));

describe("context menu in the parameter list", () => {
  it("right-clicking a row (or its control) opens the param menu: show the automation lane, reset", async () => {
    mock = await renderWithMock(
      <>
        <DeviceChain />
        <ContextMenuHost />
      </>,
    );
    const device = await insertMega(mock);
    const card = await screen.findByRole("region", { name: "Mock Mega" }, { timeout: 5000 });
    const list = await showParams(card);
    const row = within(list).getByText("Gain 0").closest<HTMLElement>("[data-param-row]")!;
    const param = Number(row.dataset.paramRow);
    fireEvent.contextMenu(row.querySelector(".eth-params__control > *") ?? row);
    fireEvent.click(await screen.findByRole("menuitem", { name: "show automation lane" }));
    const ui = useAutomationUi.getState();
    expect(JSON.stringify(ui)).toContain(targetKey({ type: "DeviceParam", device: device.id, param }));
  });
});

describe("card selection", () => {
  it("takes pins, then the plugin's quick controls, then the first automatable params", () => {
    expect(MEGA.params).toHaveLength(MOCK_MEGA_PARAM_COUNT);
    const plain = cardParams(MEGA.params).map((p) => p.id);
    expect(plain).toHaveLength(PARAM_CARD_CAP);
    // The 8 remote controls first (in slot order), then automatable visible params.
    expect(plain.slice(0, 8)).toEqual([0, 1, 2, 3, 4, 5, 6, 7].map((i) => MOCK_MEGA_REMOTE_FIRST + i));
    const rest = plain.slice(8);
    expect(rest.every((id) => MEGA.params[id]!.automatable && !MEGA.params[id]!.hidden)).toBe(true);
    expect(rest[0]).toBe(0);

    const pinned = cardParams(MEGA.params, [4241, 9000 /* hidden */, 12]).map((p) => p.id);
    expect(pinned.slice(0, 3)).toEqual([4241, 12, MOCK_MEGA_REMOTE_FIRST]);
    expect(pinned).toHaveLength(PARAM_CARD_CAP);
    expect(new Set(pinned).size).toBe(PARAM_CARD_CAP);
  });

  it("caps plugins only above the cap; built-ins only above UNCAPPED_MAX", () => {
    const capped = resolveLayout(MEGA);
    expect(capped.capped).toBe(true);
    expect(capped.total).toBe(VISIBLE);
    const small: DeviceDescriptor = { ...MEGA, params: MEGA.params.slice(0, PARAM_CARD_CAP) };
    expect(resolveLayout(small).capped).toBe(false);
    const builtin: DeviceDescriptor = { ...MEGA, device_type: { type: "Builtin", device: "Utility" }, params: MEGA.params.slice(0, 40) };
    expect(resolveLayout(builtin).capped).toBe(false);
  });
});

describe("10,000-param plugin in the chain", () => {
  it("the card mounts no control until its parameters are shown; the list mounts only the rows in view", async () => {
    const card = await renderMega();
    expect(cardParamIds(card)).toHaveLength(0);
    const list = await showParams(card);
    const rows = () => within(list).queryAllByRole("listitem");
    expect(rows().length).toBeGreaterThan(5);
    expect(rows().length).toBeLessThanOrEqual(VIRTUAL_MAX_ROWS);
    expect(within(list).getByText("Gain 0")).toBeTruthy();
    // The area is capped (a design token) and the list scrolls inside it.
    expect(list.closest(".eth-device-params")).not.toBeNull();
    expect(list).toHaveClass("eth-vlist");

    // Scrolling mounts the rows around the new position (and unmounts the first ones).
    const rowPx = Number(getComputedStyle(list.querySelector<HTMLElement>(".eth-vlist__row")!).height.replace("px", ""));
    expect(rowPx).toBeGreaterThan(0);
    act(() => {
      list.scrollTop = rowPx * 5_000;
      fireEvent.scroll(list);
    });
    expect(within(list).queryByText("Gain 0")).toBeNull();
    const names = rows().map((r) => r.textContent ?? "");
    expect(names.some((n) => / 50\d\d(?!\d)/.test(n))).toBe(true);
    expect(rows().length).toBeLessThanOrEqual(VIRTUAL_MAX_ROWS);
  });

  it("searches by name and group", async () => {
    const card = await renderMega();
    await showParams(card);
    const list = () => within(card).getByRole("list", { name: "Mock Mega parameters" });
    fireEvent.change(within(card).getByRole("searchbox", { name: "search Mock Mega parameters" }), { target: { value: "cutoff 4241" } });
    await waitFor(() => expect(within(list()).getAllByRole("listitem")).toHaveLength(1));
    expect(within(list()).getByText("Cutoff 4241")).toBeTruthy();
    fireEvent.change(within(card).getByRole("searchbox", { name: "search Mock Mega parameters" }), { target: { value: "no such param" } });
    await waitFor(() => expect(within(list()).queryAllByRole("listitem")).toHaveLength(0));
    expect(within(card).getByText(/no parameter matches/)).toBeTruthy();
  });

  it("pins a param from the inline list: it comes first (pins are per plugin)", async () => {
    const card = await renderMega();
    await showParams(card);
    // A new search remounts the list (back to the top).
    const list = () => within(card).getByRole("list", { name: "Mock Mega parameters" });
    const firstRow = () => list().querySelector<HTMLElement>("[data-param-row]")!.dataset.paramRow;
    expect(firstRow()).toBe("0");
    fireEvent.change(within(card).getByRole("searchbox", { name: "search Mock Mega parameters" }), { target: { value: "cutoff 4241" } });
    await waitFor(() => expect(within(list()).getAllByRole("listitem")).toHaveLength(1));
    fireEvent.click(within(list()).getByRole("button", { name: "pin Cutoff 4241" }));
    expect(getPins(KEY)).toEqual([4241]);
    fireEvent.change(within(card).getByRole("searchbox", { name: "search Mock Mega parameters" }), { target: { value: "" } });
    await waitFor(() => expect(firstRow()).toBe("4241"));
    expect(within(list()).getByRole("button", { name: "unpin Cutoff 4241" })).toHaveAttribute("aria-pressed", "true");
  });

  it("rows follow their own param's value", async () => {
    const card = await renderMega();
    const device = Object.values(store().project!.devices).find((d) => d.name === "Mock Mega")!;
    const list = await showParams(card);
    const row = () => list.querySelector<HTMLElement>('[data-param-row="6"]')!; // "Mix 6", %
    expect(row().textContent).toContain("100");
    await act(async () => {
      await mock!.send(cmd("Device", { type: "SetParam", device: device.id, param: 6, value: 25 }));
    });
    await flush();
    expect(row().textContent).toContain("25");
  });
});

describe("10,000-param generic layout card", () => {
  function renderCards(...ids: string[]) {
    const transport = new MockTransport({ timers: "manual", seed: 1 });
    const r = render(
      <TransportProvider transport={transport}>
        {ids.map((id) => (
          <Card key={id} device={megaDevice(id)} descriptor={MEGA} />
        ))}
      </TransportProvider>,
    );
    return {
      cards: screen.getAllByRole("region", { name: "perf card" }),
      done: () => {
        r.unmount();
        transport.dispose();
        useProjectStore.getState().reset();
      },
    };
  }

  it("mounts at most the cap of controls, and the full list only mounts its window", () => {
    const { cards, done } = renderCards("mega-1");
    const card = cards[0]!;
    expect(cardParamIds(card).length).toBeGreaterThan(0);
    expect(cardParamIds(card).length).toBeLessThanOrEqual(PARAM_CARD_CAP);
    // Expanding "More controls" still stays within the cap.
    fireEvent.click(within(card).getByRole("button", { name: /more Mock Mega controls/ }));
    const ids = cardParamIds(card);
    expect(ids).toHaveLength(PARAM_CARD_CAP);
    expect(ids).toContain(MOCK_MEGA_REMOTE_FIRST);
    // The whole card is small: a few elements per control.
    expect(card.querySelectorAll("*").length).toBeLessThan(PARAM_CARD_CAP * 20 + 60);

    fireEvent.click(within(card).getByRole("button", { name: `show all ${VISIBLE} Mock Mega parameters` }));
    const list = screen.getByRole("list", { name: "Mock Mega parameters" });
    const rows = within(list).queryAllByRole("listitem");
    expect(rows.length).toBeGreaterThan(5);
    expect(rows.length).toBeLessThanOrEqual(VIRTUAL_MAX_ROWS);
    done();
  });

  it("the all-parameters dialog has a visible close (#578)", async () => {
    const { cards, done } = renderCards("mega-1");
    fireEvent.click(within(cards[0]!).getByRole("button", { name: /^show all/ }));
    const dialog = screen.getByRole("dialog", { name: /all parameters/ });
    fireEvent.click(within(dialog).getByRole("button", { name: "close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    done();
  });

  it("pins a param to the card, per plugin, kept across reloads", async () => {
    const { cards, done } = renderCards("mega-1", "mega-2");
    const card = cards[0]!;
    fireEvent.click(within(card).getByRole("button", { name: /^show all/ }));
    const list = () => screen.getByRole("list", { name: "Mock Mega parameters" });
    fireEvent.change(screen.getByRole("searchbox", { name: "search Mock Mega parameters" }), { target: { value: "cutoff 4241" } });
    await waitFor(() => expect(within(list()).getAllByRole("listitem")).toHaveLength(1));
    fireEvent.click(within(list()).getByRole("button", { name: "pin Cutoff 4241" }));
    expect(within(list()).getByRole("button", { name: "unpin Cutoff 4241" }).getAttribute("aria-pressed")).toBe("true");
    expect(getPins(KEY)).toEqual([4241]);
    // Every card of the plugin shows it first (the main, always-visible section).
    await waitFor(() => expect(cards.every((c) => cardParamIds(c)[0] === 4241)).toBe(true));
    // Stored in UI settings (localStorage), not in the project document.
    expect(JSON.parse(localStorage.getItem("eth.devices.paramPins")!)).toEqual({ [KEY]: [4241] });

    // "Reload": re-read storage.
    reloadPins();
    expect(getPins(KEY)).toEqual([4241]);
    expect(pinKey(megaDevice("x"))).toBe(KEY);

    act(() => setPinned(KEY, 4241, false));
    await waitFor(() => expect(cardParamIds(card)[0]).toBe(MOCK_MEGA_REMOTE_FIRST));
    done();
  });
});

/**
 * Milliseconds of this thread's CPU time (Node's `process.threadCpuUsage`): the scripting cost
 * itself, not the wall time a busy machine (parallel suites, other jobs) adds by descheduling
 * the worker. Wall time where it is missing (older Node).
 */
function scriptingClock(): () => number {
  const cpu = (globalThis as { process?: { threadCpuUsage?: () => { user: number; system: number } } }).process?.threadCpuUsage;
  if (!cpu) return () => performance.now();
  return () => {
    const u = cpu();
    return (u.user + u.system) / 1000;
  };
}

describe("performance", () => {
  const device = megaDevice("perf-device");
  const now = scriptingClock();

  it("opens a 10,000-param card and its full list in well under 100 ms of scripting", () => {
    const transport = new MockTransport({ timers: "manual", seed: 1 });
    const mount = () =>
      render(
        <TransportProvider transport={transport}>
          <Card device={device} descriptor={MEGA} />
        </TransportProvider>,
      );
    // Warm up (module init, JIT: card and list), then measure fresh descriptors (no memo
    // carried over). Best of 5: the suite runs files in parallel on busy machines, and the
    // minimum is the scripting cost without scheduling noise.
    const warm = mount();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /^show all/ }));
    });
    warm.unmount();
    let card = Infinity;
    let open = Infinity;
    for (let i = 0; i < 5; i++) {
      const fresh: DeviceDescriptor = { ...MEGA, params: [...MEGA.params] };
      const t0 = now();
      const r = render(
        <TransportProvider transport={transport}>
          <Card device={device} descriptor={fresh} />
        </TransportProvider>,
      );
      card = Math.min(card, now() - t0);
      expect(r.container.querySelectorAll("[data-param]").length).toBeLessThanOrEqual(PARAM_CARD_CAP);
      const t1 = now();
      act(() => {
        fireEvent.click(screen.getByRole("button", { name: /^show all/ }));
      });
      open = Math.min(open, now() - t1);
      expect(screen.getByRole("list", { name: "Mock Mega parameters" })).toBeTruthy();
      r.unmount();
    }
    expect(card).toBeLessThan(100);
    expect(open).toBeLessThan(100);
    transport.dispose();
    useProjectStore.getState().reset();
  });
});
