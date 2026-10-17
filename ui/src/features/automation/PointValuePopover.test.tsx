import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ParamInfo } from "@/generated";
import { SILENCE_DB } from "@/domain/paramScale";
import { CLIP_TRANSPOSE_INFO, DEVICE_ON_INFO, editText, editUnit, PAN_INFO, parseEditText, VOLUME_INFO } from "./params";
import { PointValuePopover, type PointValueEdit } from "./PointValuePopover";

const info = (unit: ParamInfo["unit"], min: number, max: number, extra: Partial<ParamInfo> = {}): ParamInfo => ({
  id: 1,
  name: "P",
  group: null,
  unit,
  min,
  max,
  default: min,
  scale: { type: "Linear" },
  labels: null,
  automatable: true,
  hidden: false,
  ...extra,
});

describe("typed values", () => {
  it("shows the value in the param's display unit", () => {
    expect([editText(VOLUME_INFO, -6), editUnit(VOLUME_INFO)]).toEqual(["-6", "db"]);
    expect(editText(VOLUME_INFO, SILENCE_DB)).toBe("-inf");
    expect([editText(PAN_INFO, -0.25), editUnit(PAN_INFO)]).toEqual(["25L", ""]);
    expect(editText(DEVICE_ON_INFO, 1)).toBe("On");
    expect([editText(info("Hertz", 20, 20000), 1234.567), editUnit(info("Hertz", 20, 20000))]).toEqual(["1234.57", "hz"]);
    expect(editText(info("None", 0, 3, { labels: ["Sine", "Saw", "Square", "Noise"] }), 2)).toBe("Square");
  });

  it("parses display forms and bare numbers, clamped to the range", () => {
    expect(parseEditText(VOLUME_INFO, "-3.5")).toBe(-3.5);
    expect(parseEditText(VOLUME_INFO, " -12 db ")).toBe(-12);
    expect(parseEditText(VOLUME_INFO, "-inf")).toBe(SILENCE_DB);
    expect(parseEditText(VOLUME_INFO, "20")).toBe(6);
    expect(parseEditText(PAN_INFO, "25L")).toBe(-0.25);
    expect(parseEditText(PAN_INFO, "R 40")).toBe(0.4);
    expect(parseEditText(PAN_INFO, "c")).toBe(0);
    expect(parseEditText(PAN_INFO, "-50")).toBe(-0.5);
    expect(parseEditText(DEVICE_ON_INFO, "off")).toBe(0);
    expect(parseEditText(CLIP_TRANSPOSE_INFO, "+7 st")).toBe(7);
    const hz = info("Hertz", 20, 20000);
    expect(parseEditText(hz, "1.2k")).toBe(1200);
    expect(parseEditText(hz, "1,5 khz")).toBe(1500);
    expect(parseEditText(info("Milliseconds", 0, 5000), "1.5s")).toBe(1500);
    expect(parseEditText(info("None", 0, 3, { labels: ["Sine", "Saw", "Square", "Noise"] }), "saw")).toBe(1);
  });

  it("rejects text that isn't a value", () => {
    expect(parseEditText(VOLUME_INFO, "")).toBeNull();
    expect(parseEditText(VOLUME_INFO, "loud")).toBeNull();
    expect(parseEditText(VOLUME_INFO, "3 hz")).toBeNull();
  });
});

describe("PointValuePopover", () => {
  const edit: PointValueEdit = { ids: ["a", "b"], plain: -6, x: 40, y: 20 };

  function setup() {
    const onApply = vi.fn();
    const onDone = vi.fn();
    const view = render(<PointValuePopover edit={edit} info={VOLUME_INFO} onApply={onApply} onDone={onDone} />);
    const input = screen.getByRole("textbox", { name: "volume value" }) as HTMLInputElement;
    return { onApply, onDone, input, view };
  }

  it("opens at the point, focused with the value in dB selected", () => {
    const { input } = setup();
    expect(screen.getByRole("dialog", { name: "set volume" })).toBeTruthy();
    expect(input.value).toBe("-6");
    expect(document.activeElement).toBe(input);
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 2]);
    expect(screen.getByText("db")).toBeTruthy();
    const anchor = document.querySelector<HTMLElement>(".eth-auto-lane__value-anchor")!;
    expect([anchor.style.left, anchor.style.top]).toEqual(["40px", "20px"]);
  });

  it("Enter applies the typed value to every point", () => {
    const { input, onApply, onDone } = setup();
    fireEvent.change(input, { target: { value: "-12.5 db" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onApply).toHaveBeenCalledExactlyOnceWith(["a", "b"], -12.5);
    expect(onDone).toHaveBeenCalledOnce();
  });

  it("Enter on text that isn't a value keeps it open, marked invalid", () => {
    const { input, onApply, onDone } = setup();
    fireEvent.change(input, { target: { value: "loud" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onApply).not.toHaveBeenCalled();
    expect(onDone).not.toHaveBeenCalled();
    expect(input.getAttribute("aria-invalid")).toBe("true");
  });

  it("Escape cancels, and keys don't reach the lane's shortcuts", () => {
    const onLaneKey = vi.fn();
    const onApply = vi.fn();
    const onDone = vi.fn();
    render(
      <div onKeyDown={onLaneKey}>
        <PointValuePopover edit={edit} info={VOLUME_INFO} onApply={onApply} onDone={onDone} />
      </div>,
    );
    const input = screen.getByRole("textbox", { name: "volume value" });
    fireEvent.change(input, { target: { value: "-1" } });
    fireEvent.keyDown(input, { key: "Backspace" });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(onApply).not.toHaveBeenCalled();
    expect(onDone).toHaveBeenCalledOnce();
    expect(onLaneKey).not.toHaveBeenCalled();
  });

  it("a click outside applies (like the other inline editors)", () => {
    const { input, onApply, onDone } = setup();
    fireEvent.change(input, { target: { value: "0" } });
    act(() => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(onApply).toHaveBeenCalledExactlyOnceWith(["a", "b"], 0);
    expect(onDone).toHaveBeenCalledOnce();
  });
});
