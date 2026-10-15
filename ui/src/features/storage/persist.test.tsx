import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useNotices } from "@/features/notifications";
import { ensurePersistentStorage, NOTICE_KEY, requestPersistence, useStorageStatus, type StorageApi } from "./persist";
import { StorageSettings } from "./StorageSettings";

function api(answers: { persisted?: boolean; persist?: boolean[]; throws?: boolean } = {}) {
  const queue = [...(answers.persist ?? [])];
  return {
    persisted: vi.fn(() => Promise.resolve(answers.persisted ?? false)),
    persist: vi.fn(() => (answers.throws ? Promise.reject(new Error("nope")) : Promise.resolve(queue.shift() ?? false))),
    estimate: vi.fn(() => Promise.resolve({ usage: 5 * 1024 * 1024, quota: 2 * 1024 ** 3 })),
  } satisfies StorageApi;
}

const notices = () => useNotices.getState().notices.map((n) => n.title);
const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

beforeEach(() => {
  localStorage.clear();
  useNotices.getState().clear();
  useStorageStatus.setState({ persistence: "unknown", usage: null, quota: null });
});
afterEach(() => vi.unstubAllGlobals());

describe("persistent storage (#622)", () => {
  it("already persistent: nothing asked, no notice", async () => {
    const a = api({ persisted: true });
    await ensurePersistentStorage({ api: a, target: window });
    expect(a.persist).not.toHaveBeenCalled();
    expect(useStorageStatus.getState()).toMatchObject({ persistence: "persistent", usage: 5 * 1024 * 1024 });
    expect(notices()).toEqual([]);
  });

  it("granted on launch: persistent, no notice", async () => {
    const a = api({ persist: [true] });
    await ensurePersistentStorage({ api: a, target: window });
    expect(a.persist).toHaveBeenCalledOnce();
    expect(useStorageStatus.getState().persistence).toBe("persistent");
    expect(notices()).toEqual([]);
  });

  it("refused: asked again on the first click; granted then, no notice", async () => {
    const a = api({ persist: [false, true] });
    await ensurePersistentStorage({ api: a, target: window });
    expect(useStorageStatus.getState().persistence).toBe("best-effort");
    fireEvent.pointerDown(window);
    await flush();
    expect(a.persist).toHaveBeenCalledTimes(2);
    expect(useStorageStatus.getState().persistence).toBe("persistent");
    expect(notices()).toEqual([]);
  });

  it("refused twice: the notice, once; it opens the settings", async () => {
    const onOpenSettings = vi.fn();
    const a = api({ persist: [false, false, false, false] });
    await ensurePersistentStorage({ api: a, target: window, onOpenSettings });
    fireEvent.pointerDown(window);
    fireEvent.pointerDown(window);
    await flush();
    expect(a.persist).toHaveBeenCalledTimes(2);
    expect(notices()).toEqual(["projects may be cleared"]);
    useNotices.getState().notices[0]!.onClick?.();
    expect(onOpenSettings).toHaveBeenCalledOnce();
    expect(localStorage.getItem(NOTICE_KEY)).toBe("1");

    // The next launch: refused again, no notice.
    useNotices.getState().clear();
    await ensurePersistentStorage({ api: a, target: window });
    fireEvent.pointerDown(window);
    await flush();
    expect(notices()).toEqual([]);
  });

  it("unsupported or failing: never throws; unsupported says so at once", async () => {
    await expect(ensurePersistentStorage({ api: {}, target: window })).resolves.toBeUndefined();
    expect(useStorageStatus.getState().persistence).toBe("unsupported");
    expect(notices()).toEqual(["projects may be cleared"]);
    await expect(requestPersistence(api({ throws: true }))).resolves.toBe("best-effort");
  });

  it("settings: the state, the usage and request again", async () => {
    const a = api({ persist: [true] });
    vi.stubGlobal("navigator", { ...navigator, storage: a });
    useStorageStatus.setState({ persistence: "best-effort" });
    render(<StorageSettings />);
    await flush();
    expect(screen.getByRole("status")).toHaveTextContent("storage: may be cleared by the browser when space runs low");
    expect(screen.getByText(/using 5 MB of 2 GB/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "request again" }));
    await flush();
    expect(a.persist).toHaveBeenCalledOnce();
    expect(screen.getByRole("status")).toHaveTextContent("storage: persistent: the browser keeps your projects");
    expect(screen.queryByRole("button", { name: "request again" })).toBeNull();
  });
});
