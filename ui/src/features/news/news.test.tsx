import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, URL } from "node:url";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_NEWS_URL, detectOs, listUrl, resolveNewsEndpoint, type NewsEnv } from "./config";
import { paletteEntries, setPaletteSource } from "@/features/keymap";
import { runMenuAction } from "@/features/desktop-menu";
import { buildCommands } from "@/app/shell/commands";
import { NEWS_SANDBOX, parseNewsMessage } from "./messages";
import { NewsRoot, NewsSettings } from "./NewsRoot";
import { newsCommands } from "./index";
import { checkNews, resetNews, SETTINGS_KEY, useNews, type FetchOptions, type NewsItem } from "./store";
import { useTour } from "@/features/tutorials/store";
import type { TourDef } from "@/features/tutorials/types";

const ENV: NewsEnv = { dev: false, automated: false, override: "https://news.test" };
const item = (id: string, day: number): NewsItem => ({
  id,
  title: `Item ${id}`,
  published_at: `2026-10-${String(day).padStart(2, "0")}T00:00:00Z`,
  html_url: `https://news.test/v1/news/${id}/content?v=1`,
});

let fetcher: ReturnType<typeof vi.fn>;
let items: NewsItem[];
let opts: FetchOptions;

beforeEach(() => {
  localStorage.clear();
  resetNews();
  items = [item("older", 1), item("newer", 9)];
  fetcher = vi.fn(async () => new Response(JSON.stringify({ items }), { status: 200 }));
  opts = { fetcher: fetcher as unknown as typeof fetch, env: ENV };
});

afterEach(() => {
  resetNews();
});

const mount = () => render(<NewsRoot delayMs={0} fetchOptions={opts} />);

describe("news at launch", () => {
  it("fetches once, shows the newest item in a sandboxed iframe, and a dismissed item never comes back", async () => {
    const { unmount } = mount();
    const frame = await screen.findByTestId("news-frame");
    expect(fetcher).toHaveBeenCalledTimes(1);
    const [url, init] = fetcher.mock.calls[0] as [string, RequestInit];
    expect(url).toMatch(/^https:\/\/news\.test\/v1\/news\?version=[^&]+&platform=web&os=/);
    expect(init.credentials).toBe("omit");
    expect(frame.getAttribute("src")).toBe(items[1]!.html_url);
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts allow-popups allow-popups-to-escape-sandbox");
    expect(frame.getAttribute("sandbox")).not.toContain("allow-same-origin");
    expect(screen.getByText("Item newer")).toBeTruthy();
    // Two items: "Next" dismisses this one and shows the other.
    fireEvent.click(screen.getByTestId("news-next"));
    expect(screen.getByTestId("news-frame").getAttribute("src")).toBe(items[0]!.html_url);
    fireEvent.click(screen.getByTestId("news-close"));
    expect(useNews.getState().current).toBeNull();
    expect(JSON.parse(localStorage.getItem(SETTINGS_KEY)!)).toEqual({ enabled: true, dismissed: ["newer", "older"] });
    unmount();

    // Next launch: nothing new, nothing shown.
    resetNews();
    mount();
    await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(2));
    await act(async () => {});
    expect(screen.queryByTestId("news-frame")).toBeNull();
  });

  it("closes with Escape and the header ×; items not seen yet stay for the next launch", async () => {
    mount();
    await screen.findByTestId("news-frame");
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(useNews.getState().current).toBeNull());
    expect(useNews.getState().dismissed).toEqual(["newer"]);

    resetNews(); // as on the next launch
    await act(() => checkNews(opts));
    expect(useNews.getState().current?.id).toBe("older");
    fireEvent.click(await screen.findByTestId("news-close-x"));
    expect(useNews.getState().dismissed).toEqual(["newer", "older"]);
  });

  it("waits while a guided tour runs, then shows; a tour started over it hides it until the tour ends (#623)", async () => {
    const tourOn = () => act(() => useTour.setState({ tour: { id: "probe", steps: [] } as unknown as TourDef }));
    const tourOff = () => act(() => useTour.setState({ tour: null }));
    try {
      // A tour first: the item is fetched but not shown over it.
      tourOn();
      mount();
      await waitFor(() => expect(useNews.getState().current?.id).toBe("newer"));
      expect(screen.queryByRole("dialog")).toBeNull();
      // The tour ends: it shows, not dismissed meanwhile.
      tourOff();
      expect(await screen.findByTestId("news-frame")).toBeTruthy();
      expect(useNews.getState().dismissed).toEqual([]);
      // News first, then a tour starts: the news steps aside (still not dismissed) and comes back.
      tourOn();
      await waitFor(() => expect(screen.queryByTestId("news-frame")).toBeNull());
      // Escape during the tour is the tour's, not the hidden news'.
      fireEvent.keyDown(window, { key: "Escape" });
      expect(useNews.getState().current?.id).toBe("newer");
      tourOff();
      expect(await screen.findByTestId("news-frame")).toBeTruthy();
      expect(useNews.getState().dismissed).toEqual([]);
    } finally {
      act(() => useTour.setState({ tour: null }));
    }
  });

  it("does not fetch when checking for news is off", async () => {
    render(<NewsSettings />);
    fireEvent.click(screen.getByRole("switch", { name: "check for news" }));
    expect(JSON.parse(localStorage.getItem(SETTINGS_KEY)!).enabled).toBe(false);
    resetNews(); // as on the next launch
    expect(useNews.getState().enabled).toBe(false);
    await checkNews(opts);
    expect(fetcher).not.toHaveBeenCalled();
    expect(useNews.getState().current).toBeNull();
  });

  it("is silent when offline or the answer is malformed", async () => {
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    fetcher.mockRejectedValueOnce(new TypeError("Failed to fetch"));
    await checkNews(opts);
    fetcher.mockResolvedValueOnce(new Response("nope", { status: 200 }));
    await checkNews(opts);
    fetcher.mockResolvedValueOnce(new Response(JSON.stringify({ items: [{ id: "x", title: "t", published_at: "d", html_url: "javascript:alert(1)" }] })));
    await checkNews(opts);
    expect(useNews.getState().current).toBeNull();
    expect(info).toHaveBeenCalledTimes(2);
    // A server error and a service that never answers (the timeout aborts it): still silent.
    fetcher.mockResolvedValueOnce(new Response("down", { status: 503 }));
    await checkNews(opts);
    fetcher.mockImplementationOnce(
      (_url: string, init: RequestInit) =>
        new Promise((_, reject) => init.signal!.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")))),
    );
    await checkNews({ ...opts, timeoutMs: 5 });
    expect(useNews.getState().current).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(info).toHaveBeenCalledTimes(4);
    info.mockRestore();
  });

  it('lists past items from "News…", dismissed ones included, and reopens one without changing anything', async () => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify({ enabled: true, dismissed: ["newer", "older"] }));
    resetNews();
    render(<NewsRoot delayMs={60_000} fetchOptions={opts} />);
    const cmd = newsCommands()[0]!;
    expect(cmd.label).toBe("news…");
    const { openNewsList } = await import("./store");
    act(() => openNewsList(opts));
    const entry = await screen.findByRole("button", { name: /Item older/ });
    expect(screen.getByRole("button", { name: /Item newer/ })).toBeTruthy();
    fireEvent.click(entry);
    expect(screen.getByTestId("news-frame").getAttribute("src")).toBe(items[0]!.html_url);
    expect(screen.queryByTestId("news-next")).toBeNull();
    fireEvent.click(screen.getByTestId("news-close"));
    expect(useNews.getState().dismissed).toEqual(["newer", "older"]);
  });
});

describe("the page's messages", () => {
  it("accepts only close and resize from its own iframe", async () => {
    mount();
    const frame = (await screen.findByTestId("news-frame")) as HTMLIFrameElement;
    // Another window's message is ignored.
    act(() => window.dispatchEvent(new MessageEvent("message", { data: { type: "ethereal-news", action: "close" }, origin: "null", source: window })));
    expect(useNews.getState().current).not.toBeNull();
    act(() =>
      window.dispatchEvent(
        new MessageEvent("message", { data: { type: "ethereal-news", action: "resize", height: 9999 }, origin: "null", source: frame.contentWindow }),
      ),
    );
    expect(frame.style.height).toBe("640px");
    act(() =>
      window.dispatchEvent(
        new MessageEvent("message", { data: { type: "ethereal-news", action: "close" }, origin: "https://evil.test", source: frame.contentWindow }),
      ),
    );
    expect(useNews.getState().current).not.toBeNull();
    act(() =>
      window.dispatchEvent(new MessageEvent("message", { data: { type: "ethereal-news", action: "close" }, origin: "null", source: frame.contentWindow })),
    );
    expect(useNews.getState().current).toBeNull();
  });

  it("parses nothing else", () => {
    expect(parseNewsMessage({ type: "ethereal-news", action: "open", url: "file:///" })).toBeNull();
    expect(parseNewsMessage({ type: "ethereal-news", action: "resize", height: "1" })).toBeNull();
    expect(parseNewsMessage("close")).toBeNull();
    expect(parseNewsMessage({ type: "ethereal-news", action: "close", extra: 1 })).toEqual({ type: "ethereal-news", action: "close" });
  });
});

describe("config", () => {
  it("fetches nothing in dev and automated runs unless told where", () => {
    expect(resolveNewsEndpoint({ dev: false, automated: false, override: null })).toBe(DEFAULT_NEWS_URL);
    expect(resolveNewsEndpoint({ dev: true, automated: false, override: null })).toBeNull();
    expect(resolveNewsEndpoint({ dev: false, automated: true, override: null })).toBeNull();
    expect(resolveNewsEndpoint({ dev: true, automated: true, override: "http://127.0.0.1:8791/" })).toBe("http://127.0.0.1:8791");
    expect(resolveNewsEndpoint({ dev: false, automated: false, override: "" })).toBeNull();
  });

  it("shapes the list URL from the version, platform and OS, and nothing else", () => {
    const url = new URL(listUrl("https://news.test", { version: "0.6.0-dev.abc", platform: "desktop", os: "macos" }));
    expect(url.origin + url.pathname).toBe("https://news.test/v1/news");
    expect([...url.searchParams.entries()]).toEqual([
      ["version", "0.6.0-dev.abc"],
      ["platform", "desktop"],
      ["os", "macos"],
    ]);
    expect(listUrl("https://news.test", { version: "1.0.0+a&b", platform: "web", os: "other" })).toBe(
      "https://news.test/v1/news?version=1.0.0%2Ba%26b&platform=web&os=other",
    );
    // The public default needs no private build; the override strips its trailing slashes.
    expect(DEFAULT_NEWS_URL).toMatch(/^https:\/\/[^/]+$/);
    expect(resolveNewsEndpoint({ dev: false, automated: false, override: " https://n.example/// " })).toBe("https://n.example");
  });

  it("names the OS coarsely", () => {
    expect(detectOs("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe("macos");
    expect(detectOs("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows");
    expect(detectOs("Mozilla/5.0 (X11; Linux x86_64)")).toBe("linux");
    expect(detectOs("Mozilla/5.0 (Linux; Android 14)")).toBe("android");
    expect(detectOs("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)")).toBe("ios");
  });
});

describe("the help menu", () => {
  afterEach(() => setPaletteSource(null));

  it('"news…" (desktop Help menu `app:news`, the palette) opens the list', () => {
    setPaletteSource(() => buildCommands(null, []));
    expect(paletteEntries().find((e) => e.id === "app:news")?.label).toBe("news…");
    runMenuAction("app:news", null);
    expect(useNews.getState().listOpen).toBe(true);
  });
});

describe("isolation from the app", () => {
  const path = (rel: string) => fileURLToPath(new URL(rel, import.meta.url));
  const read = (rel: string) => readFileSync(path(rel), "utf8");

  it("the sandbox never grants same-origin", () => {
    expect(NEWS_SANDBOX.split(" ")).not.toContain("allow-same-origin");
    expect(NEWS_SANDBOX.split(" ")).not.toContain("allow-top-navigation");
  });

  it("the desktop shell gives IPC to no remote origin and frames only the news service", () => {
    const dir = join(dirname(path("../../../../apps/desktop/src-tauri/tauri.conf.json")), "capabilities");
    const files = readdirSync(dir).filter((f) => f.endsWith(".json"));
    expect(files.length).toBeGreaterThan(0);
    for (const f of files) {
      const capability = JSON.parse(readFileSync(join(dir, f), "utf8")) as Record<string, unknown>;
      // A `remote` entry would let a web origin (the news service included) call the shell.
      expect(capability.remote, f).toBeUndefined();
      expect(capability.windows, f).toEqual(["main"]);
    }
    const conf = JSON.parse(read("../../../../apps/desktop/src-tauri/tauri.conf.json")) as {
      app: { withGlobalTauri?: boolean; security: { csp: Record<string, string>; capabilities?: unknown } };
    };
    expect(conf.app.withGlobalTauri ?? false).toBe(false);
    expect(conf.app.security.capabilities).toBeUndefined();
    expect(conf.app.security.csp["frame-src"]).toBe(DEFAULT_NEWS_URL);
  });

  it("the web app frames only the news service", () => {
    const csp = /Content-Security-Policy:\s*(.+)/.exec(read("../../../../scripts/release/web/_headers"))![1]!;
    expect(/frame-src ([^;]+)/.exec(csp)![1]!.trim()).toBe(DEFAULT_NEWS_URL);
  });
});
