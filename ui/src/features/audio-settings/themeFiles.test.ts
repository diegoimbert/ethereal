import { describe, expect, it } from "vitest";
import { themeFileName } from "./themeFiles";

describe("theme file names", () => {
  it("are the theme's name as a plain slug", () => {
    expect(themeFileName("my theme")).toBe("my-theme.json");
    expect(themeFileName("Crépuscule d'été!")).toBe("crepuscule-d-ete.json");
    expect(themeFileName("  ")).toBe("theme.json");
  });
});
