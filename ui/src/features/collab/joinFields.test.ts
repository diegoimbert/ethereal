import { afterEach, describe, expect, it, vi } from "vitest";
import type { EngineTransport } from "@/transport";
import { relayError, sessionNameError, tokenStorage, WEB_TOKEN_KEY } from "./joinFields";

afterEach(() => localStorage.clear());

describe("session name validation (relay rule: 1-64 of [A-Za-z0-9._-])", () => {
  it("accepts valid names", () => {
    for (const ok of ["jam", "my-song_2.1", "A".repeat(64)]) expect(sessionNameError(ok)).toBeNull();
  });

  it("explains what is wrong", () => {
    expect(sessionNameError("")).toBe("Enter a session name.");
    expect(sessionNameError("A".repeat(65))).toBe("Use at most 64 characters.");
    expect(sessionNameError("my song")).toMatch(/only letters, digits/);
    expect(sessionNameError("chanson-été")).toMatch(/only letters, digits/);
    expect(sessionNameError("a/b")).toMatch(/only letters, digits/);
  });

  it("checks the relay address", () => {
    expect(relayError("ws://studio.local:9003")).toBeNull();
    expect(relayError("wss://relay.example")).toBeNull();
    expect(relayError("http://x")).toMatch(/relay address/);
  });
});

describe("remembered token", () => {
  it("web: this browser's localStorage", async () => {
    const t = tokenStorage(null);
    expect(t.where).toBe("browser");
    expect(await t.load()).toBeNull();
    await t.save("s3cret");
    expect(localStorage.getItem(WEB_TOKEN_KEY)).toBe("s3cret");
    expect(await t.load()).toBe("s3cret");
    await t.save(null);
    expect(localStorage.getItem(WEB_TOKEN_KEY)).toBeNull();
  });

  it("desktop: the app data dir through the transport, never localStorage", async () => {
    let kept: string | null = "from-disk";
    const host = {
      loadCollabToken: vi.fn(async () => kept),
      saveCollabToken: vi.fn(async (token: string | null) => {
        kept = token;
      }),
    } as unknown as EngineTransport;
    const t = tokenStorage(host);
    expect(t.where).toBe("app");
    expect(await t.load()).toBe("from-disk");
    await t.save("new");
    expect(kept).toBe("new");
    expect(localStorage.getItem(WEB_TOKEN_KEY)).toBeNull();
  });

  it("never logs the token, even when storing fails", async () => {
    const log = vi.spyOn(console, "log");
    const warn = vi.spyOn(console, "warn");
    const error = vi.spyOn(console, "error");
    const host = {
      loadCollabToken: () => Promise.reject(new Error("no")),
      saveCollabToken: () => Promise.reject(new Error("no")),
    } as unknown as EngineTransport;
    const t = tokenStorage(host);
    await t.save("top-secret");
    expect(await t.load()).toBeNull();
    for (const spy of [log, warn, error]) expect(JSON.stringify(spy.mock.calls)).not.toContain("top-secret");
  });
});
