import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Logo, LOGO_FACETS, LOGO_GRADIENTS } from "./Logo";

const source = readFileSync(resolve(__dirname, "../../../assets/brand/ethereal-logo.svg"), "utf8");

describe("Logo", () => {
  it("draws the facets and gradients of assets/brand/ethereal-logo.svg", () => {
    const paths = [...source.matchAll(/<path d="([^"]+)" fill="([^"]+)"/g)].map(([, d, fill]) => [d, fill!.replace(/^url\(#paint(\d)_linear_1_16\)$/, "g$1")]);
    expect(paths).toEqual(LOGO_FACETS);
    const gradients = [
      ...source.matchAll(/<linearGradient[^>]*x1="([\d.]+)" y1="([\d.]+)" x2="([\d.]+)" y2="([\d.]+)"[\s\S]*?stop-color="(#\w+)"[\s\S]*?stop-color="(#\w+)"/g),
    ].map(([, x1, y1, x2, y2, from, to]) => ({ x1: Number(x1), y1: Number(y1), x2: Number(x2), y2: Number(y2), from, to }));
    expect(gradients).toEqual(LOGO_GRADIENTS);
  });

  it("gives each instance its own gradient ids (two logos on a page keep their gradients)", () => {
    render(
      <>
        <Logo />
        <Logo />
      </>,
    );
    const [a, b] = screen.getAllByTestId("logo");
    const ids = (svg: Element) => Array.from(svg.querySelectorAll("linearGradient")).map((g) => g.id);
    expect(ids(a!)).toHaveLength(LOGO_GRADIENTS.length);
    expect(ids(a!).filter((id) => ids(b!).includes(id))).toEqual([]);
    // Every gradient fill points at a gradient of its own logo.
    for (const svg of [a!, b!]) {
      const own = new Set(ids(svg));
      for (const p of svg.querySelectorAll("path")) {
        const ref = /^url\(#(.+)\)$/.exec(p.getAttribute("fill")!)?.[1];
        if (ref) expect(own.has(ref)).toBe(true);
      }
    }
  });

  it("is decorative without a title, an image with one", () => {
    const { rerender } = render(<Logo />);
    expect(screen.queryByRole("img")).toBeNull();
    rerender(<Logo title="Ethereal" />);
    expect(screen.getByRole("img", { name: "Ethereal" })).toBeInTheDocument();
  });
});
