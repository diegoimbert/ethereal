import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import { pickOption } from "@/kit/testing";
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { createDemoProject } from "@/transport/mock";
import { resetRenderJobs } from "@/features/freeze/store";
import { ConvertToSamplerDialog } from "./ConvertToSamplerDialog";
import { useSamplerConvert } from "./store";

let sent: Command[];

function setup() {
  const project = createDemoProject();
  useProjectStore.setState({ project });
  const device = Object.values(project.devices).find((d) => project.tracks[d.track]?.kind === "Midi")!;
  sent = [];
  const transport = {
    kind: "mock",
    send: async (c: Command): Promise<ReplyValue> => {
      sent.push(c);
      return { type: "Unit" };
    },
    onEvent: () => () => {},
  } as unknown as EngineTransport;
  render(
    <TransportContext.Provider value={{ transport, connection: { status: "connected" } }}>
      <ConvertToSamplerDialog />
    </TransportContext.Provider>,
  );
  act(() => useSamplerConvert.getState().open(device));
}

const key = (name: string) => screen.getByRole("checkbox", { name });
const checked = (name: string) => key(name).getAttribute("aria-checked");
const picked = () =>
  screen
    .getAllByRole("checkbox")
    .filter((k) => k.getAttribute("aria-checked") === "true")
    .map((k) => k.getAttribute("aria-label"));
const velocities = () => screen.getByTestId("convert-velocities").textContent;
const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

beforeEach(() => resetRenderJobs());
afterEach(() => {
  act(() => useSamplerConvert.getState().close());
  useProjectStore.setState({ project: null });
});

describe("convert to sampler dialog", () => {
  it("starts on every 3 semitones from C1 to C6 and 3 velocities", () => {
    setup();
    expect(picked()).toHaveLength(21);
    expect(checked("c1")).toBe("true");
    expect(checked("c#1")).toBe("false");
    expect(velocities()).toBe("42 · 85 · 127");
  });

  it("a click toggles a key and switches the preset to custom", () => {
    setup();
    const preset = screen.getByRole("combobox", { name: "notes to sample" });
    fireEvent.pointerDown(key("c#1"), { button: 0 });
    fireEvent.pointerUp(window);
    expect(checked("c#1")).toBe("true");
    expect(preset.textContent).toContain("custom");
    fireEvent.pointerDown(key("c#1"), { button: 0 });
    fireEvent.pointerUp(window);
    expect(checked("c#1")).toBe("false");
  });

  it("a drag sweeps keys on, or off when it starts on a picked key", () => {
    setup();
    fireEvent.pointerDown(key("c#1"), { button: 0 });
    fireEvent.pointerEnter(key("f1"));
    fireEvent.pointerUp(window);
    expect(["c1", "c#1", "d1", "d#1", "e1", "f1"].map(checked)).toEqual(Array(6).fill("true"));
    // The sweep runs from the pressed key to the pointer: going back restores the keys past it.
    fireEvent.pointerDown(key("c1"), { button: 0 });
    fireEvent.pointerEnter(key("g1"));
    fireEvent.pointerEnter(key("d1"));
    fireEvent.pointerUp(window);
    expect(["c1", "c#1", "d1"].map(checked)).toEqual(["false", "false", "false"]);
    expect(checked("d#1")).toBe("true");
    // After the release, moving over keys changes nothing.
    fireEvent.pointerEnter(key("a1"));
    expect(checked("a1")).toBe("true");
  });

  it("a right-button drag removes keys, whatever the pressed key was", () => {
    setup();
    // c#1 is off, c1 and d#1 are on: all of c1..d#1 end up off.
    fireEvent.pointerDown(key("c#1"), { button: 2 });
    fireEvent.pointerEnter(key("d#1"));
    fireEvent.pointerEnter(key("c1"));
    fireEvent.pointerUp(window);
    expect(["c1", "c#1", "d1", "d#1"].map(checked)).toEqual(["false", "false", "false", "true"]);
    fireEvent.pointerDown(key("d#1"), { button: 2 });
    fireEvent.pointerUp(window);
    expect(checked("d#1")).toBe("false");
  });

  it("a preset spans the picked keys", () => {
    setup();
    pickOption(screen.getByRole("combobox", { name: "notes to sample" }), "every octave");
    expect(picked()).toEqual(["c1", "c2", "c3", "c4", "c5", "c6"]);
  });

  it("configure lists the velocities; editing one switches to custom", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: "configure…" }));
    const popover = screen.getByRole("dialog", { name: "velocities" });
    const field = within(popover).getByRole("spinbutton", { name: "velocity 2" });
    fireEvent.change(field, { target: { value: "100" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(velocities()).toBe("42 · 100 · 127");
    expect(screen.getByRole("combobox", { name: "velocities to render" }).textContent).toContain("custom");
    fireEvent.click(within(popover).getByRole("button", { name: "remove velocity 42" }));
    expect(velocities()).toBe("100 · 127");
    fireEvent.click(within(popover).getByRole("button", { name: "add a velocity" }));
    expect(velocities()).toBe("50 · 100 · 127");
  });

  it("converts the picked keys and velocities", async () => {
    setup();
    pickOption(screen.getByRole("combobox", { name: "notes to sample" }), "every octave");
    pickOption(screen.getByRole("combobox", { name: "velocities to render" }), "1 velocity");
    fireEvent.click(screen.getByRole("button", { name: "convert" }));
    await flush();
    const c = sent.find((c) => c.domain === "Freeze");
    expect(c?.domain === "Freeze" && c.command.type === "ConvertToSampler" && [c.command.keys, c.command.velocities]).toEqual([
      [36, 48, 60, 72, 84, 96],
      [127],
    ]);
  });

  it("can't convert with no key", () => {
    setup();
    fireEvent.pointerDown(key("c1"), { button: 0 });
    fireEvent.pointerEnter(key("c6"));
    fireEvent.pointerUp(window);
    expect(picked()).toEqual([]);
    expect(screen.getByRole("button", { name: "convert" })).toHaveProperty("disabled", true);
    expect(screen.getByText("pick at least one key to sample.")).toBeTruthy();
  });
});
