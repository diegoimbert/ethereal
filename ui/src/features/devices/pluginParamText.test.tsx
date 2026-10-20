// A plugin's knobs show the plugin's own display text (`Plugin::ParamTexts`: VST2
// `effGetParamDisplay` + label, CLAP `value_to_text`, VST3 `getParamStringByValue` + units),
// and the formatted number when the host has none.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Command, PluginParamValue, ReplyValue } from "@/generated";
import { cmd, CommandFailedError, newId, TransportProvider, type EngineTransport, type SendOptions } from "@/transport";
import { MOCK_MEGA_PLUGIN_ID, MockTransport } from "@/transport/mock";
import { mockParamText } from "@/transport/mock/plugins";
import { flush, resetStores, store, stubPointerCapture } from "./testUtils";
import { DeviceChain } from "./index";
import { ParamTextSource } from "./pluginParamText";

/** The mock engine plus a desktop-like `ParamTexts`: "<value> dB" for Gain params, none for others. */
class TextHost extends MockTransport {
  readonly asked: PluginParamValue[][] = [];
  unsupported = false;

  override send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    if (command.domain === "Plugin" && command.command.type === "ParamTexts") {
      const { values } = command.command;
      this.asked.push(values);
      if (this.unsupported) return Promise.reject(new CommandFailedError({ code: "Unsupported", message: "no plugins here" }, command));
      const texts = values.map((v) => (v.param % 8 === 0 ? `${v.value.toFixed(1)} dB (plugin)` : null));
      return Promise.resolve({ type: "ParamTexts", texts });
    }
    return super.send(command, opts);
  }
}

let host: TextHost | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  cleanup();
  resetStores(host);
  host = undefined;
});

async function renderPluginRows(): Promise<{ list: HTMLElement; device: string }> {
  host = new TextHost({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={host}>
      <DeviceChain />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind !== "Master")!;
  const id = newId();
  await act(async () => {
    await host!.send(
      cmd("Device", { type: "Insert", id, track: track.id, device: { type: "Plugin", plugin_id: MOCK_MEGA_PLUGIN_ID, sandboxed: null }, before: null }),
    );
  });
  await flush();
  const card = await screen.findByRole("region", { name: "Mock Mega" }, { timeout: 5000 });
  fireEvent.click(within(card).getByRole("button", { name: "show Mock Mega parameters" }));
  const list = await within(card).findByRole("list", { name: "Mock Mega parameters" }, { timeout: 5000 });
  return { list, device: id };
}

const valueText = (list: HTMLElement, param: number) => list.querySelector(`[data-param-row="${param}"] .eth-param__value`)?.textContent;

describe("plugin param display text", () => {
  it("knobs show the plugin's text, follow the value, and fall back to the number without one", async () => {
    const { list, device } = await renderPluginRows();
    // "Gain 8": the plugin has a text; "Cutoff 1": none (formatted by the UI).
    await waitFor(() => expect(valueText(list, 8)).toBe("0.0 dB (plugin)"));
    expect(valueText(list, 1)).toBe("0");
    // One batched command per device for every visible row.
    const first = host!.asked[0]!;
    expect(first.length).toBeGreaterThan(2);
    expect(first.find((v) => v.param === 8)).toEqual({ param: 8, value: 0 });

    await act(async () => {
      await host!.send(cmd("Device", { type: "SetParam", device, param: 8, value: -6 }));
    });
    await waitFor(() => expect(valueText(list, 8)).toBe("-6.0 dB (plugin)"));
  });

  it("a host without texts is asked once, then the UI formats every value", async () => {
    const source = new ParamTextSource(Object.assign(new TextHost({ timers: "manual", seed: 2 }), { unsupported: true }) as EngineTransport);
    const t = source as unknown as { transport: TextHost };
    await expect(source.text("d", 1, 0.5)).resolves.toBeNull();
    expect(source.unsupported).toBe(true);
    await expect(source.text("d", 2, 0.5)).resolves.toBeNull();
    expect(t.transport.asked.length).toBe(1);
    t.transport.dispose();
  });

  it("the mock engine's plugins give their own text (Cutoff as Hz)", () => {
    expect(mockParamText(MOCK_MEGA_PLUGIN_ID, 1, 0)).toBe("20 Hz");
    expect(mockParamText(MOCK_MEGA_PLUGIN_ID, 1, 0.5)).toBe("632 Hz");
    expect(mockParamText(MOCK_MEGA_PLUGIN_ID, 1, 1)).toBe("20.00 kHz");
    expect(mockParamText(MOCK_MEGA_PLUGIN_ID, 0, 0)).toBeNull();
    expect(mockParamText("nope", 1, 0)).toBeNull();
  });

  it("batches requests per device and caches texts per value", async () => {
    const h = new TextHost({ timers: "manual", seed: 3 });
    const source = new ParamTextSource(h);
    const [a, b, c] = await Promise.all([source.text("d1", 0, 1), source.text("d1", 3, 1), source.text("d2", 8, 2)]);
    expect([a, b, c]).toEqual(["1.0 dB (plugin)", null, "2.0 dB (plugin)"]);
    expect(h.asked).toEqual([
      [
        { param: 0, value: 1 },
        { param: 3, value: 1 },
      ],
      [{ param: 8, value: 2 }],
    ]);
    // Cached: no new command. A value without text is asked again (VST2: not applied yet).
    await expect(source.text("d1", 0, 1)).resolves.toBe("1.0 dB (plugin)");
    await source.text("d1", 3, 1);
    expect(h.asked.length).toBe(3);
    h.dispose();
  });
});
