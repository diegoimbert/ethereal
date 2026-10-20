// eth-allow-case-file: third-party plugin names (test fixtures).
// Test helpers shared by the plugin and device-library tests: plugin descriptors and the
// desktop host as seen by the UI.
import { act } from "@testing-library/react";
import type { Command, Event, PluginDescriptor, PluginEvent, ReplyValue } from "@/generated";
import { Emitter, type EngineTransport, type SendOptions, type Unsubscribe } from "@/transport";
import { MockTransport } from "@/transport/mock";

export const plugin = (
  id: string,
  name: string,
  category: PluginDescriptor["category"],
  vendor = "Acme",
  format: PluginDescriptor["format"] = "Clap",
): PluginDescriptor => ({
  format,
  id,
  name,
  vendor,
  version: "1.0",
  description: "",
  features: category === "Instrument" ? ["instrument", "synthesizer"] : ["audio-effect", "reverb"],
  category,
  path: `/plugins/${name}.clap`,
  sidechain_inputs: 0,
});

export const VERB = plugin("com.acme.verb", "Verb", "AudioEffect");
export const SYNTH = plugin("com.acme.synth", "Big Synth", "Instrument");
export const EQ = plugin("org.other.eq", "Air EQ", "AudioEffect", "Other");
export const VST_VERB = plugin("E7E1E4A1000000000000000000000001", "Verb", "AudioEffect", "Acme", "Vst3");
export const AU_DELAY = plugin("aufx:dely:appl", "AUDelay", "AudioEffect", "Apple", "Au");
/** The same plugin also installed as VST2: listed separately, told apart by its chip. */
export const VST2_VERB = plugin("41636D56", "Verb", "AudioEffect", "Acme", "Vst2");

/** The desktop host as seen by the UI: MockTransport's document + host-handled plugin commands. */
export class DesktopFake implements EngineTransport {
  readonly kind = "tauri" as const;
  readonly mock = new MockTransport({ timers: "manual", seed: 3 });
  readonly events = new Emitter<Event>();
  readonly sent: Command[] = [];
  plugins: PluginDescriptor[] = [VERB, SYNTH];

  connect() {
    return this.mock.connect();
  }
  async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    if (command.domain === "Plugin") {
      this.sent.push(command);
      if (command.command.type === "List") return { type: "Plugins", plugins: this.plugins };
      return { type: "Unit" };
    }
    if (command.domain === "Device" && command.command.type === "Insert" && command.command.device.type === "Plugin") {
      this.sent.push(command);
      return { type: "Unit" };
    }
    // An instrument replacing the track's instrument: one batch holding the plugin Insert.
    const isPluginInsert = (c: Command) => c.domain === "Device" && c.command.type === "Insert" && c.command.device.type === "Plugin";
    if (command.domain === "Edit" && command.command.type === "Batch" && command.command.commands.some(isPluginInsert)) {
      this.sent.push(command);
      return { type: "Unit" };
    }
    return this.mock.send(command, opts);
  }
  onEvent(listener: (event: Event) => void): Unsubscribe {
    const a = this.mock.onEvent(listener);
    const b = this.events.on(listener);
    return () => {
      a();
      b();
    };
  }
  subscribePlayhead: EngineTransport["subscribePlayhead"] = (l) => this.mock.subscribePlayhead(l);
  subscribeMeters: EngineTransport["subscribeMeters"] = (l) => this.mock.subscribeMeters(l);
  dispose() {
    this.mock.dispose();
  }
  emit(event: PluginEvent) {
    act(() => this.events.emit({ type: "Plugin", event }));
  }
}
