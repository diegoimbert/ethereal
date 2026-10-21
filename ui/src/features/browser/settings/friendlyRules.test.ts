import { describe, expect, it } from "vitest";
import { fromRegex, toRegex } from "./friendlyRules";

describe("friendly file rules", () => {
  it("reads the default rules in plain terms", () => {
    expect(fromRegex("\\.(wav|wave|aif|aiff|flac)$")).toEqual({ kind: "type", value: "wav, wave, aif, aiff, flac" });
    expect(fromRegex("\\.(mid|midi)$")).toEqual({ kind: "type", value: "mid, midi" });
    expect(fromRegex("^\\.")).toEqual({ kind: "starts", value: "." });
    expect(fromRegex("^Thumbs\\.db$")).toEqual({ kind: "is", value: "Thumbs.db" });
    expect(fromRegex("^desktop\\.ini$")).toEqual({ kind: "is", value: "desktop.ini" });
    expect(fromRegex("\\.asd$")).toEqual({ kind: "type", value: "asd" });
    expect(fromRegex("draft")).toEqual({ kind: "contains", value: "draft" });
    expect(fromRegex("_old$")).toEqual({ kind: "ends", value: "_old" });
  });

  it("leaves real patterns as custom", () => {
    expect(fromRegex("^take[0-9]+")).toBeNull();
    expect(fromRegex("a.*b")).toBeNull();
    expect(fromRegex("")).toBeNull();
  });

  it("writes them back as the same rules", () => {
    for (const rule of ["\\.(wav|aif)$", "\\.asd$", "^\\.", "^Thumbs\\.db$", "draft", "_old$"]) {
      expect(toRegex(fromRegex(rule)!)).toBe(rule);
    }
    expect(toRegex({ kind: "type", value: ".wav, aif ,flac" })).toBe("\\.(wav|aif|flac)$");
    expect(toRegex({ kind: "contains", value: "a+b" })).toBe("a\\+b");
    expect(toRegex({ kind: "type", value: " , " })).toBe("");
  });
});
