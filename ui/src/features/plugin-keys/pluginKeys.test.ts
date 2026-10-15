import { describe, expect, it, vi } from "vitest";
import type { Command, Event } from "@/generated";
import type { EngineTransport } from "@/transport";
import { matchesAction } from "@/features/keymap";
import { asKeyboardEvent, chordCodes, onPluginWindowKey, requestedCaptureCodes, setEditorKeyCapture } from "./index";

function fakeTransport() {
  const sent: Command[] = [];
  const listeners = new Set<(e: Event) => void>();
  const transport = {
    send: vi.fn((c: Command) => {
      sent.push(c);
      return Promise.reject(new Error("Unsupported"));
    }),
    onEvent: (fn: (e: Event) => void) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
  } as unknown as EngineTransport;
  return { transport, sent, emit: (e: Event) => listeners.forEach((l) => l(e)) };
}

const space = { key: " ", code: "Space", shift: false, ctrl: false, alt: false, meta: false, down: true, repeat: false };

describe("plugin-keys", () => {
  it("unions every owner's codes into one command, sent only on change, Space never sent", async () => {
    const { transport, sent } = fakeTransport();
    setEditorKeyCapture(transport, "transport", ["Space"]);
    expect(sent).toEqual([]); // Space is always captured by the host
    setEditorKeyCapture(transport, "keyboard-midi", ["KeyA", "KeyW"]);
    setEditorKeyCapture(transport, "transport", ["KeyP", "Space"]);
    setEditorKeyCapture(transport, "transport", ["Space", "KeyP"]); // same set: nothing sent
    expect(sent).toEqual([
      { domain: "Plugin", command: { type: "SetEditorKeyCapture", codes: ["KeyA", "KeyW"] } },
      { domain: "Plugin", command: { type: "SetEditorKeyCapture", codes: ["KeyA", "KeyP", "KeyW"] } },
    ]);
    setEditorKeyCapture(transport, "keyboard-midi", []);
    expect(requestedCaptureCodes(transport)).toEqual(["KeyP"]);
    expect(sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "SetEditorKeyCapture", codes: ["KeyP"] } });
    // The web has no plugin host: the rejection is swallowed.
    await Promise.resolve();
  });

  it("delivers forwarded keys, readable by keymap matching", () => {
    const { transport, emit } = fakeTransport();
    const got: unknown[] = [];
    const off = onPluginWindowKey(transport, (k, device) => got.push([k.code, device]));
    emit({ type: "Plugin", event: { type: "EditorKey", device: "d1", key: space } });
    emit({ type: "Plugin", event: { type: "EditorClosed", device: "d1" } });
    off();
    emit({ type: "Plugin", event: { type: "EditorKey", device: "d1", key: space } });
    expect(got).toEqual([["Space", "d1"]]);
    expect(matchesAction("transport.play", asKeyboardEvent(space))).toBe(true);
    expect(matchesAction("transport.stopToSelection", asKeyboardEvent({ ...space, shift: true }))).toBe(true);
    expect(asKeyboardEvent({ ...space, down: false }).type).toBe("keyup");
  });

  it("chordCodes: plain / Shift chords only", () => {
    expect(chordCodes(["Space", "Shift+Space", "P", "Mod+P", "Alt+K", "1", "[", "F9"])).toEqual(["BracketLeft", "Digit1", "KeyP", "Space"]);
  });
});
