// base-132: a plugin with 10,000 params never renders all of them. The card shows at most
// PARAM_CARD_CAP controls (pins → the plugin's quick controls → first automatable params);
// "Show all N parameters…" opens a searchable list that mounts only the rows in view; pins
// persist per plugin in UI settings.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { Device, DeviceDescriptor } from "@/generated";
import { VIRTUAL_MAX_ROWS } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, MOCK_MEGA_PARAM_COUNT, MOCK_MEGA_PLUGIN_ID, MockTransport, newId, TransportProvider } from "@/transport";
import { mockPluginDescriptor, MOCK_MEGA_REMOTE_FIRST } from "@/transport/mock/plugins";
import { flush, renderWithMock, resetStores, store, stubPointerCapture } from "@/features/mixer/testUtils";
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
    await m.send(cmd("Device", { type: "Insert", id, track: firstTrack().id, device: { type: "Plugin", plugin_id: MOCK_MEGA_PLUGIN_ID, sandboxed: null }, before: null }));
  });
  await flush();
  return store().project!.devices[id]!;
}

async function renderMega(): Promise<HTMLElement> {
  mock = await renderWithMock(<DeviceChain />);
  await insertMega(mock);
  return screen.findByRole("region", { name: "Mock Mega" }, { timeout: 5000 });
}

const cardParamIds = (card: HTMLElement) => [...card.querySelectorAll<HTMLElement>("[data-param]")].map((e) => Number(e.dataset.param));

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

describe("10,000-param plugin card", () => {
  it("mounts at most the cap of controls, and the full list only mounts its window", async () => {
    const card = await renderMega();
    await waitFor(() => expect(cardParamIds(card).length).toBeGreaterThan(0));
    expect(cardParamIds(card).length).toBeLessThanOrEqual(PARAM_CARD_CAP);
    // Expanding "More controls" still stays within the cap.
    fireEvent.click(within(card).getByRole("button", { name: /More Mock Mega controls/ }));
    const ids = cardParamIds(card);
    expect(ids).toHaveLength(PARAM_CARD_CAP);
    expect(ids).toContain(MOCK_MEGA_REMOTE_FIRST);
    // The whole card is small: a few elements per control.
    expect(card.querySelectorAll("*").length).toBeLessThan(PARAM_CARD_CAP * 20 + 60);

    fireEvent.click(within(card).getByRole("button", { name: `Show all ${VISIBLE} Mock Mega parameters` }));
    const list = await screen.findByRole("list", { name: "Mock Mega parameters" });
    const rows = () => within(list).queryAllByRole("listitem");
    expect(rows().length).toBeGreaterThan(5);
    expect(rows().length).toBeLessThanOrEqual(VIRTUAL_MAX_ROWS);
    expect(within(list).getByText("Gain 0")).toBeTruthy();

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
    fireEvent.click(await within(card).findByRole("button", { name: /^Show all \d+ Mock Mega parameters$/ }));
    await screen.findByRole("list", { name: "Mock Mega parameters" });
    const list = () => screen.getByRole("list", { name: "Mock Mega parameters" });
    fireEvent.change(screen.getByRole("searchbox", { name: "Search Mock Mega parameters" }), { target: { value: "cutoff 4241" } });
    await waitFor(() => expect(within(list()).getAllByRole("listitem")).toHaveLength(1));
    expect(within(list()).getByText("Cutoff 4241")).toBeTruthy();
    fireEvent.change(screen.getByRole("searchbox", { name: "Search Mock Mega parameters" }), { target: { value: "no such param" } });
    await waitFor(() => expect(within(list()).queryAllByRole("listitem")).toHaveLength(0));
    expect(screen.getByText(/No parameter matches/)).toBeTruthy();
  });

  it("pins a param to the card, per plugin, kept across reloads", async () => {
    const card = await renderMega();
    fireEvent.click(await within(card).findByRole("button", { name: /^Show all/ }));
    await screen.findByRole("list", { name: "Mock Mega parameters" });
    const list = () => screen.getByRole("list", { name: "Mock Mega parameters" });
    fireEvent.change(screen.getByRole("searchbox", { name: "Search Mock Mega parameters" }), { target: { value: "cutoff 4241" } });
    await waitFor(() => expect(within(list()).getAllByRole("listitem")).toHaveLength(1));
    fireEvent.click(within(list()).getByRole("button", { name: "Pin Cutoff 4241" }));
    expect(within(list()).getByRole("button", { name: "Unpin Cutoff 4241" }).getAttribute("aria-pressed")).toBe("true");
    expect(getPins(KEY)).toEqual([4241]);
    // The card shows it first (the main, always-visible section).
    await waitFor(() => expect(cardParamIds(card)[0]).toBe(4241));
    // Stored in UI settings (localStorage), not in the project document.
    expect(JSON.parse(localStorage.getItem("eth.devices.paramPins")!)).toEqual({ [KEY]: [4241] });
    expect(JSON.stringify(store().project)).not.toContain("4241");

    // "Reload": re-read storage, and a second instance of the plugin shows the pin too.
    reloadPins();
    expect(getPins(KEY)).toEqual([4241]);
    const second = await insertMega(mock!);
    expect(pinKey(second)).toBe(KEY);
    const cards = await screen.findAllByRole("region", { name: "Mock Mega" });
    await waitFor(() => expect(cards.every((c) => cardParamIds(c)[0] === 4241)).toBe(true));

    setPinned(KEY, 4241, false);
    await waitFor(() => expect(cardParamIds(card)[0]).toBe(MOCK_MEGA_REMOTE_FIRST));
  });

  it("rows follow their own param's value", async () => {
    const card = await renderMega();
    const device = Object.values(store().project!.devices).find((d) => d.name === "Mock Mega")!;
    fireEvent.click(await within(card).findByRole("button", { name: /^Show all/ }));
    const list = await screen.findByRole("list", { name: "Mock Mega parameters" });
    const row = () => list.querySelector<HTMLElement>('[data-param-row="6"]')!; // "Mix 6", %
    expect(row().textContent).toContain("100");
    await act(async () => {
      await mock!.send(cmd("Device", { type: "SetParam", device: device.id, param: 6, value: 25 }));
    });
    await flush();
    expect(row().textContent).toContain("25");
  });
});

describe("performance", () => {
  function Card({ device, descriptor }: { device: Device; descriptor: DeviceDescriptor }) {
    const sender = useGestureSender();
    return (
      <section aria-label="perf card">
        <DeviceLayoutView device={device} descriptor={descriptor} sender={sender} />
      </section>
    );
  }
  const device: Device = {
    id: "perf-device",
    track: "t",
    order: "a",
    name: "Mock Mega",
    enabled: true,
    kind: { type: "Plugin", plugin: { format: "Clap", plugin_id: MOCK_MEGA_PLUGIN_ID, name: "Mock Mega", vendor: "", version: "", sandboxed: false, state: null } },
    params: {},
    sidechain: null,
    pad: null,
  };

  it("opens a 10,000-param card and its full list in well under 100 ms of scripting", () => {
    const transport = new MockTransport({ timers: "manual", seed: 1 });
    const mount = () =>
      render(
        <TransportProvider transport={transport}>
          <Card device={device} descriptor={MEGA} />
        </TransportProvider>,
      );
    // Warm up (module init, JIT: card and list), then measure a fresh descriptor (no memo
    // carried over).
    const warm = mount();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /^Show all/ }));
    });
    warm.unmount();
    const fresh: DeviceDescriptor = { ...MEGA, params: [...MEGA.params] };
    const t0 = performance.now();
    const r = render(
      <TransportProvider transport={transport}>
        <Card device={device} descriptor={fresh} />
      </TransportProvider>,
    );
    const card = performance.now() - t0;
    expect(r.container.querySelectorAll("[data-param]").length).toBeLessThanOrEqual(PARAM_CARD_CAP);
    expect(card).toBeLessThan(100);

    const t1 = performance.now();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /^Show all/ }));
    });
    const open = performance.now() - t1;
    expect(screen.getByRole("list", { name: "Mock Mega parameters" })).toBeTruthy();
    expect(open).toBeLessThan(100);
    r.unmount();
    transport.dispose();
    useProjectStore.getState().reset();
  });
});

