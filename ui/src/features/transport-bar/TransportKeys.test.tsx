// transport-keys: Space always plays / stops, Shift+Space returns to the selection, the Stop
// button's double press, keys forwarded from plugin editor windows.
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, Event } from "@/generated";
import { playheadStore, useProjectStore } from "@/state";
import { resetTimeSelection, useTimeSelection } from "@/features/time-edits/store";
import { pauseDispatch } from "@/features/keymap/dispatcher";
import { cmd } from "@/transport";
import { renderWithMock, resetStores } from "./testUtils";
import { blurAfterPointer, installTransportKeys, keepsTransportKey, routeTransportKey, type TransportKeyAction } from "./keys";
import { TransportBar } from "./index";

const playing = () => useProjectStore.getState().transport?.playing ?? false;
const playhead = () => playheadStore.getPlayhead()?.transport.position ?? 0;

function el(html: string): HTMLElement {
  const host = document.createElement("div");
  host.innerHTML = html;
  document.body.append(host);
  return host.firstElementChild as HTMLElement;
}

afterEach(() => {
  document.body.innerHTML = "";
  resetStores();
  resetTimeSelection();
});

describe("routing rule (who gets Space)", () => {
  it("text entry keeps Space; buttons, checkboxes, sliders and closed selects don't", () => {
    expect(keepsTransportKey(el("<input type='text'>"))).toBe(true);
    expect(keepsTransportKey(el("<input>"))).toBe(true);
    expect(keepsTransportKey(el("<input type='search'>"))).toBe(true);
    expect(keepsTransportKey(el("<input type='number'>"))).toBe(true);
    expect(keepsTransportKey(el("<textarea></textarea>"))).toBe(true);
    const editable = el("<div contenteditable='true'></div>");
    // jsdom has no isContentEditable: emulate the browser.
    Object.defineProperty(editable, "isContentEditable", { value: true });
    expect(keepsTransportKey(editable)).toBe(true);

    expect(keepsTransportKey(el("<button>Mute</button>"))).toBe(false);
    expect(keepsTransportKey(el("<input type='checkbox'>"))).toBe(false);
    expect(keepsTransportKey(el("<input type='range'>"))).toBe(false);
    expect(keepsTransportKey(el("<select><option>a</option></select>"))).toBe(false);
    expect(keepsTransportKey(el("<button role='combobox' aria-expanded='false'>Grid</button>"))).toBe(false);
    expect(keepsTransportKey(el("<input type='text' readonly>"))).toBe(false);
    expect(keepsTransportKey(document.body)).toBe(false);
    expect(keepsTransportKey(null)).toBe(false);
  });

  it("only text boxes are exempt: menus, listboxes (rack chains), open comboboxes, switches and dialogs play too", () => {
    expect(keepsTransportKey(el("<div role='menu'><button role='menuitem'>Cut</button></div>").firstElementChild)).toBe(false);
    expect(keepsTransportKey(el("<ul role='listbox'><li role='option' tabindex='0'>x</li></ul>").firstElementChild)).toBe(false);
    expect(keepsTransportKey(el("<div role='listbox'><div role='option'><button>Mute Chain 1</button></div></div>").querySelector("button"))).toBe(false);
    expect(keepsTransportKey(el("<button role='combobox' aria-expanded='true'>Grid</button>"))).toBe(false);
    expect(keepsTransportKey(el("<button role='switch' aria-checked='false'>On</button>"))).toBe(false);
    expect(keepsTransportKey(el("<div role='slider' tabindex='0'></div>"))).toBe(false);
    expect(keepsTransportKey(el("<div role='dialog'><button>OK</button></div>").firstElementChild)).toBe(false);
    for (const t of ["radio", "button", "submit", "color", "file"]) expect(keepsTransportKey(el(`<input type='${t}'>`))).toBe(false);
    // Text boxes, wherever they are.
    expect(keepsTransportKey(el("<div role='dialog'><input type='text'></div>").firstElementChild)).toBe(true);
    expect(keepsTransportKey(el("<div role='listbox'><input role='combobox' aria-expanded='true'></div>").firstElementChild)).toBe(true);
    for (const t of ["email", "url", "tel", "password"]) expect(keepsTransportKey(el(`<input type='${t}'>`))).toBe(true);
    expect(keepsTransportKey(el("<div role='textbox' tabindex='0'></div>"))).toBe(true);
    expect(keepsTransportKey(el("<div role='textbox' aria-readonly='true' tabindex='0'></div>"))).toBe(false);
    expect(keepsTransportKey(el("<div contenteditable='true'><span>x</span></div>").firstElementChild)).toBe(true);
    expect(keepsTransportKey(el("<textarea readonly></textarea>"))).toBe(false);
  });

  it("a text field inside a shadow root keeps Space (deepest target)", () => {
    const host = el("<div></div>");
    const input = document.createElement("input");
    host.attachShadow({ mode: "open" }).append(input);
    const e = { key: " ", code: "Space", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false };
    expect(routeTransportKey({ ...e, target: host, composedPath: () => [input, host] })).toBeNull();
    const button = document.createElement("button");
    host.shadowRoot!.append(button);
    expect(routeTransportKey({ ...e, target: host, composedPath: () => [button, host] })).toBe("transport.play");
  });

  it("maps Space to play and Shift+Space to stop-to-selection (keymap actions)", () => {
    const button = el("<button>x</button>");
    expect(routeTransportKey({ key: " ", code: "Space", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, target: button })).toBe(
      "transport.play",
    );
    expect(routeTransportKey({ key: " ", code: "Space", shiftKey: true, ctrlKey: false, altKey: false, metaKey: false, target: button })).toBe(
      "transport.stopToSelection",
    );
    expect(routeTransportKey({ key: " ", code: "Space", shiftKey: false, ctrlKey: false, altKey: false, metaKey: true, target: button })).toBeNull();
    expect(routeTransportKey({ key: "a", code: "KeyA", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, target: button })).toBeNull();
    const input = el("<input type='text'>");
    expect(routeTransportKey({ key: " ", code: "Space", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, target: input })).toBeNull();
  });
});

describe("installTransportKeys", () => {
  let runs: TransportKeyAction[];
  let paused: boolean;
  let uninstall: () => void;
  beforeEach(() => {
    runs = [];
    paused = false;
    uninstall = installTransportKeys({ enabled: () => true, paused: () => paused, run: (a) => runs.push(a) });
  });
  afterEach(() => uninstall());

  it("runs before a focused button sees the key, and swallows the keyup it clicks on", () => {
    const button = el("<button>Mute</button>");
    const onKeyDown = vi.fn();
    button.addEventListener("keydown", onKeyDown);
    button.focus();
    const down = fireEvent.keyDown(button, { key: " ", code: "Space" });
    expect(down).toBe(false); // default prevented
    expect(onKeyDown).not.toHaveBeenCalled();
    const up = fireEvent.keyUp(button, { key: " ", code: "Space" });
    expect(up).toBe(false);
    expect(runs).toEqual(["transport.play"]);
  });

  it("plays once from any non-text control and the control never activates (keydown, keyup, click)", () => {
    const targets = [
      el("<button>Mute</button>"),
      el("<input type='checkbox'>"),
      el("<div role='slider' tabindex='0' aria-valuenow='0'></div>"),
      el("<div role='switch' tabindex='0' aria-checked='false'></div>"),
      el("<div role='menu'><button role='menuitem'>Cut</button></div>").firstElementChild as HTMLElement,
      el("<div role='listbox'><div role='option' tabindex='0'>Chain 1</div></div>").firstElementChild as HTMLElement,
      el("<div role='dialog'><button>OK</button></div>").firstElementChild as HTMLElement,
    ];
    for (const t of targets) {
      runs = [];
      const own = vi.fn();
      t.addEventListener("keydown", own);
      t.addEventListener("keyup", own);
      t.addEventListener("click", own);
      t.focus();
      expect(fireEvent.keyDown(t, { key: " ", code: "Space" })).toBe(false);
      expect(fireEvent.keyDown(t, { key: " ", code: "Space", repeat: true })).toBe(false); // held: swallowed, not re-run
      expect(fireEvent.keyUp(t, { key: " ", code: "Space" })).toBe(false);
      expect(runs).toEqual(["transport.play"]);
      expect(own).not.toHaveBeenCalled();
    }
  });

  it("text entry gets its space untouched (no transport, keyup not swallowed)", () => {
    for (const t of [el("<input type='text'>"), el("<textarea></textarea>"), el("<div contenteditable='true'></div>")]) {
      const own = vi.fn();
      t.addEventListener("keydown", own);
      t.addEventListener("keyup", own);
      expect(fireEvent.keyDown(t, { key: " ", code: "Space" })).toBe(true);
      expect(fireEvent.keyUp(t, { key: " ", code: "Space" })).toBe(true);
      expect(own).toHaveBeenCalledTimes(2);
    }
    expect(runs).toEqual([]);
  });

  it("ignores auto-repeat, text fields, and the keymap editor recording a chord", () => {
    const button = el("<button>x</button>");
    fireEvent.keyDown(button, { key: " ", code: "Space", repeat: true });
    expect(runs).toEqual([]);
    const input = el("<input type='text'>");
    expect(fireEvent.keyDown(input, { key: " ", code: "Space" })).toBe(true);
    expect(runs).toEqual([]);
    paused = true;
    expect(fireEvent.keyDown(button, { key: " ", code: "Space" })).toBe(true);
    expect(runs).toEqual([]);
  });
});

describe("blurAfterPointer", () => {
  it("hands focus to the focusable container after a mouse click, not after a keyboard one", () => {
    const off = blurAfterPointer();
    const box = el("<div tabindex='-1' class='box'><button>Mute</button></div>");
    const button = box.querySelector("button")!;
    button.focus();
    fireEvent.click(button, { detail: 1 });
    expect(document.activeElement).toBe(box);
    button.focus();
    fireEvent.click(button, { detail: 0 });
    expect(document.activeElement).toBe(button);
    // Popup triggers keep it.
    const trigger = el("<button aria-haspopup='menu'>More</button>");
    trigger.focus();
    fireEvent.click(trigger, { detail: 1 });
    expect(document.activeElement).toBe(trigger);
    off();
  });
});

describe("TransportBar keys", () => {
  async function setup() {
    const r = await renderWithMock(<TransportBar />);
    await waitFor(() => expect(screen.getByRole("button", { name: "play" })).toBeEnabled());
    const sent: Command[] = [];
    const send = r.mock.send.bind(r.mock);
    vi.spyOn(r.mock, "send").mockImplementation((c, o) => {
      sent.push(c);
      return send(c, o);
    });
    return { ...r, sent };
  }
  const flush = () => act(async () => Promise.resolve());

  it("Space on a focused button toggles the transport and does not press the button", async () => {
    await setup();
    const loop = screen.getByRole("button", { name: "loop" });
    const before = useProjectStore.getState().transport!.loop_enabled;
    loop.focus();
    fireEvent.keyDown(loop, { key: " ", code: "Space" });
    fireEvent.keyUp(loop, { key: " ", code: "Space" });
    await waitFor(() => expect(playing()).toBe(true));
    expect(useProjectStore.getState().transport!.loop_enabled).toBe(before);
    fireEvent.keyDown(loop, { key: " ", code: "Space" });
    await waitFor(() => expect(playing()).toBe(false));
  });

  it("a text field still gets its spaces", async () => {
    await setup();
    const tempo = screen.getByLabelText("tempo");
    expect(fireEvent.keyDown(tempo, { key: " ", code: "Space" })).toBe(true);
    await flush();
    expect(playing()).toBe(false);
  });

  it("Space plays and stops back to where playback started; Shift+Space plays and stops in place", async () => {
    const { mock } = await setup();
    await act(async () => {
      await mock.send(cmd("Transport", { type: "Locate", position: 8 }));
    });
    fireEvent.keyDown(window, { key: " ", code: "Space" });
    await waitFor(() => expect(playing()).toBe(true));
    act(() => mock.tick(2000));
    // No selection: Space goes back to where playback started.
    fireEvent.keyDown(window, { key: " ", code: "Space" });
    await waitFor(() => expect(playing()).toBe(false));
    await waitFor(() => expect(playhead()).toBe(8));

    // With a selection (an insert marker at 12), Space stops there.
    fireEvent.keyDown(window, { key: " ", code: "Space" });
    await waitFor(() => expect(playing()).toBe(true));
    act(() => useTimeSelection.getState().setSelection({ start: 12, end: 12, tracks: [] }));
    fireEvent.keyDown(window, { key: " ", code: "Space" });
    await waitFor(() => expect(playing()).toBe(false));
    await waitFor(() => expect(playhead()).toBe(12));

    // Shift+Space starts playback too, and stops where the playhead is.
    fireEvent.keyDown(window, { key: " ", code: "Space", shiftKey: true });
    await waitFor(() => expect(playing()).toBe(true));
    act(() => mock.tick(2000));
    fireEvent.keyDown(window, { key: " ", code: "Space", shiftKey: true });
    await waitFor(() => expect(playing()).toBe(false));
    await flush();
    expect(playhead()).toBeGreaterThan(8);
  });

  it("Stop: stops; stopped, back to the selection; there, back to the start", async () => {
    const { mock } = await setup();
    act(() => useTimeSelection.setState({ cursor: 8 }));
    fireEvent.click(screen.getByRole("button", { name: "play" }));
    await waitFor(() => expect(playing()).toBe(true));
    act(() => mock.tick(2000));
    const stop = () => fireEvent.click(screen.getAllByRole("button", { name: "stop" }).find((b) => b.getAttribute("aria-pressed") === null)!);
    stop();
    await waitFor(() => expect(playing()).toBe(false));
    const stoppedAt = playhead();
    expect(stoppedAt).toBeGreaterThan(0);
    stop();
    await waitFor(() => expect(playhead()).toBe(8));
    stop();
    await waitFor(() => expect(playhead()).toBe(0));
  });

  it("Space / Shift+Space from a plugin editor window drive the transport", async () => {
    const { mock } = await setup();
    const emit = (shift: boolean, down = true) =>
      act(() =>
        (mock as unknown as { emit(e: Event): void }).emit({
          type: "Plugin",
          event: { type: "EditorKey", device: "dev", key: { key: " ", code: "Space", shift, ctrl: false, alt: false, meta: false, down, repeat: false } },
        }),
      );
    emit(false);
    await waitFor(() => expect(playing()).toBe(true));
    emit(false, false); // the release does nothing
    await flush();
    expect(playing()).toBe(true);
    act(() => mock.tick(2000));
    // Shift+Space: stop in place.
    emit(true);
    await waitFor(() => expect(playing()).toBe(false));
    await flush();
    expect(playhead()).toBeGreaterThan(0);
  });

  it("a keymap editor recording a chord gets Space", async () => {
    await setup();
    const resume = pauseDispatch();
    fireEvent.keyDown(window, { key: " ", code: "Space" });
    await flush();
    expect(playing()).toBe(false);
    resume();
  });
});
