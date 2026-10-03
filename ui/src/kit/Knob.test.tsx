import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Knob } from "./Knob";
import { CENTER_MIN_SCALE, centerParts, centerRoom, fitScale } from "./knobFit";

describe("Knob center value", () => {
  it("splits the unit off the number", () => {
    expect(centerParts("-18.0 dB")).toEqual({ value: "-18.0", unit: "dB" });
    expect(centerParts("1.20 kHz")).toEqual({ value: "1.20", unit: "kHz" });
    expect(centerParts(" -inf dB ")).toEqual({ value: "-inf", unit: "dB" });
    expect(centerParts("Off")).toEqual({ value: "Off", unit: null });
    expect(centerParts("1/16")).toEqual({ value: "1/16", unit: null });
    // More than one space: not a "number unit" pair, keep it whole.
    expect(centerParts("50 % wet")).toEqual({ value: "50 % wet", unit: null });
  });

  it("scales text down to the room, never up, never below the floor", () => {
    expect(fitScale(30, 20)).toBe(1);
    expect(fitScale(30, 40)).toBeCloseTo(0.75);
    expect(fitScale(10, 100)).toBe(CENTER_MIN_SCALE);
    // Unmeasured (jsdom, hidden): leave it alone.
    expect(fitScale(0, 0)).toBe(1);
    expect(fitScale(30, 0)).toBe(1);
  });

  it("room: the ring's inner chord for the value, the arc's bottom gap for the unit", () => {
    // The smallest device "large" knob (34 px), 3 px stroke, 12 px tall text.
    const small = centerRoom(34, 3, 12);
    // Inner radius = 17 * 0.84 - 3 = 11.28; digits ≈ 7.2 px tall, chord at 3.6 px =
    // 2 * sqrt(11.28² - 3.6²) ≈ 21.4.
    expect(small.value).toBeCloseTo(21.4, 1);
    // Arc ends at ±29.7 % of the width: 0.594 * 34 - 6 ≈ 14.2.
    expect(small.unit).toBeCloseTo(14.2, 1);
    // Bigger dials have more room; a dial too small for the stroke has none.
    const big = centerRoom(54, 3, 12);
    expect(big.value).toBeGreaterThan(small.value);
    expect(big.unit).toBeGreaterThan(small.unit);
    expect(centerRoom(4, 3, 12)).toEqual({ value: 0, unit: 0 });
  });

  it("large knobs render the number and unit apart; small knobs don't render a center", () => {
    const { container } = render(
      <>
        <Knob value={0.2} size="lg" label="Gain" valueText="-18.0 dB" />
        <Knob value={0.2} size="lg" label="Rate" valueText="1/16" />
        <Knob value={0.2} size="sm" label="Mix" valueText="40 %" />
      </>,
    );
    const [gain, rate, mix] = container.querySelectorAll<HTMLElement>(".eth-knob");
    expect(gain!.querySelector(".eth-knob__center-value")!.textContent).toBe("-18.0");
    expect(gain!.querySelector(".eth-knob__center-unit")!.textContent).toBe("dB");
    // Not laid out (jsdom): no measurement, the CSS default scale (1) applies.
    expect(gain!.querySelector<HTMLElement>(".eth-knob__center-value")!.style.getPropertyValue("--knob-center-scale")).toBe("");
    expect(rate!.querySelector(".eth-knob__center-value")!.textContent).toBe("1/16");
    expect(rate!.querySelector(".eth-knob__center-unit")).toBeNull();
    expect(mix!.querySelector(".eth-knob__center")).toBeNull();
    // The accessible value text is unchanged.
    expect(gain).toHaveAttribute("aria-valuetext", "-18.0 dB");
  });
});
