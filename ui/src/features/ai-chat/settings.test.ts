import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clampIterations,
  DEFAULT_MAX_ITERATIONS,
  DEFAULT_MODEL,
  looksLikeApiKey,
  maskKey,
  reloadAiSettings,
  useAiSettings,
} from "./settings";

const KEY = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123";

beforeEach(() => {
  localStorage.clear();
  reloadAiSettings();
});
afterEach(() => vi.restoreAllMocks());

describe("AI settings / key storage", () => {
  it("starts without a key, with the default model and cap", () => {
    expect(useAiSettings.getState()).toMatchObject({ apiKey: null, model: DEFAULT_MODEL, maxIterations: DEFAULT_MAX_ITERATIONS });
    expect(DEFAULT_MODEL).toBe("claude-sonnet-5-5");
  });

  it("stores the key locally (trimmed), reloads it, and removes it", () => {
    useAiSettings.getState().setApiKey(`  ${KEY}\n`);
    expect(useAiSettings.getState().apiKey).toBe(KEY);
    expect(localStorage.getItem("eth.ai.apiKey")).toBe(KEY);
    useAiSettings.setState({ apiKey: null });
    reloadAiSettings();
    expect(useAiSettings.getState().apiKey).toBe(KEY);

    useAiSettings.getState().setApiKey(null);
    expect(localStorage.getItem("eth.ai.apiKey")).toBeNull();
    reloadAiSettings();
    expect(useAiSettings.getState().apiKey).toBeNull();
  });

  it("an empty key is no key", () => {
    useAiSettings.getState().setApiKey("   ");
    expect(useAiSettings.getState().apiKey).toBeNull();
    expect(localStorage.getItem("eth.ai.apiKey")).toBeNull();
  });

  it("keeps the key apart from the other settings, and never logs it", () => {
    const log = vi.spyOn(console, "log");
    const warn = vi.spyOn(console, "warn");
    useAiSettings.getState().setApiKey(KEY);
    useAiSettings.getState().setModel("claude-opus-5-5");
    useAiSettings.getState().setMaxIterations(10);
    expect(localStorage.getItem("eth.ai.settings")).not.toContain("sk-ant");
    reloadAiSettings();
    expect(useAiSettings.getState()).toMatchObject({ model: "claude-opus-5-5", maxIterations: 10 });
    for (const spy of [log, warn]) for (const c of spy.mock.calls) expect(JSON.stringify(c)).not.toContain(KEY);
  });

  it("survives storage that throws (private mode)", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("QuotaExceeded");
    });
    useAiSettings.getState().setApiKey(KEY);
    expect(useAiSettings.getState().apiKey).toBe(KEY);
  });

  it("ignores an unknown saved model and clamps the cap", () => {
    localStorage.setItem("eth.ai.settings", JSON.stringify({ model: "gpt-9", maxIterations: 9999 }));
    reloadAiSettings();
    expect(useAiSettings.getState()).toMatchObject({ model: DEFAULT_MODEL, maxIterations: 200 });
    expect(clampIterations(0)).toBe(1);
    expect(clampIterations(Number.NaN)).toBe(DEFAULT_MAX_ITERATIONS);
  });

  it("validates and masks keys for display", () => {
    expect(looksLikeApiKey(KEY)).toBe(true);
    expect(looksLikeApiKey("hello")).toBe(false);
    expect(maskKey(KEY)).toBe("sk-ant-…0123");
    expect(maskKey(KEY)).not.toContain("abcdef");
  });
});
