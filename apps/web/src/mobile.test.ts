import { afterEach, describe, expect, it, vi } from "vitest";
import { isDesktopMachine, isMobileDevice, mobileUiRequested, preferPlaybackAudioSession, wantsMobileUi, type DeviceInfo } from "./mobile";

const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605 Mobile/15E148 Safari/604.1";
const PIXEL = "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/140 Mobile Safari/537.36";
const IPAD = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605 Safari/605";
const DESKTOP = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/140 Safari/537.36";
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/140 Safari/537.36";
const ANDROID_TABLET = "Mozilla/5.0 (Linux; Android 15; SM-X910) AppleWebKit/537.36 Chrome/140 Safari/537.36";

const dev = (over: Partial<DeviceInfo>): DeviceInfo => ({
  userAgent: DESKTOP,
  screenWidth: 1920,
  screenHeight: 1080,
  coarsePointer: false,
  anyFinePointer: true,
  ...over,
});

describe("isMobileDevice (sizes x pointers matrix)", () => {
  it("blocks phones, in either orientation", () => {
    expect(isMobileDevice(dev({ userAgent: IPHONE, screenWidth: 390, screenHeight: 844, coarsePointer: true, anyFinePointer: false }))).toBe(true);
    expect(isMobileDevice(dev({ userAgent: IPHONE, screenWidth: 844, screenHeight: 390, coarsePointer: true, anyFinePointer: false }))).toBe(true);
    expect(isMobileDevice(dev({ userAgent: PIXEL, screenWidth: 412, screenHeight: 915, coarsePointer: true, anyFinePointer: false }))).toBe(true);
  });

  it("blocks a phone that misreports its pointers (desktop site: small screen or phone user agent)", () => {
    expect(isMobileDevice(dev({ userAgent: DESKTOP, screenWidth: 412, screenHeight: 915, anyFinePointer: false }))).toBe(true);
    expect(isMobileDevice(dev({ userAgent: PIXEL, screenWidth: 1200, screenHeight: 800, anyFinePointer: false }))).toBe(true);
  });

  it("blocks touch-only tablets of any size, lets tablets with a keyboard / trackpad through", () => {
    const touchOnly = { userAgent: IPAD, screenWidth: 820, screenHeight: 1180, coarsePointer: true, anyFinePointer: false };
    expect(isMobileDevice(dev(touchOnly))).toBe(true);
    expect(isMobileDevice(dev({ ...touchOnly, screenWidth: 1024, screenHeight: 1366 }))).toBe(true);
    expect(isMobileDevice(dev({ ...touchOnly, anyFinePointer: true }))).toBe(false);
  });

  it("never blocks desktops, small windows or touchscreen laptops (a fine pointer exists)", () => {
    expect(isMobileDevice(dev({}))).toBe(false);
    expect(isMobileDevice(dev({ screenWidth: 1366, screenHeight: 768 }))).toBe(false);
    // A 1366x768 laptop at 150% scaling: its screen is 911x512 CSS px, and it has a touchpad.
    expect(isMobileDevice(dev({ screenWidth: 911, screenHeight: 512 }))).toBe(false);
    expect(isMobileDevice(dev({ coarsePointer: true }))).toBe(false);
  });

  it("the size threshold: under 600 on the short side is a phone (without a fine pointer)", () => {
    const noMouse = { anyFinePointer: false };
    expect(isMobileDevice(dev({ ...noMouse, screenWidth: 599, screenHeight: 1200 }))).toBe(true);
    expect(isMobileDevice(dev({ ...noMouse, screenWidth: 600, screenHeight: 1200 }))).toBe(false);
    // No screen size known: not decided by size.
    expect(isMobileDevice(dev({ ...noMouse, screenWidth: 0, screenHeight: 0 }))).toBe(false);
  });
});

describe("isDesktopMachine (the Open on desktop band)", () => {
  it("desktops and laptops, touchscreen ones included", () => {
    expect(isDesktopMachine(dev({}))).toBe(true);
    expect(isDesktopMachine(dev({ userAgent: MAC, maxTouchPoints: 0 }))).toBe(true);
    expect(isDesktopMachine(dev({ screenWidth: 911, screenHeight: 512, maxTouchPoints: 10 }))).toBe(true);
  });

  it("never phones", () => {
    expect(isDesktopMachine(dev({ userAgent: IPHONE, screenWidth: 390, screenHeight: 844, coarsePointer: true, anyFinePointer: false }))).toBe(false);
  });

  it("never tablets, even with a keyboard and trackpad", () => {
    // iPadOS asks for the desktop site: a Mac user agent, but a touchscreen.
    expect(isDesktopMachine(dev({ userAgent: IPAD, screenWidth: 1024, screenHeight: 1366, maxTouchPoints: 5 }))).toBe(false);
    expect(isDesktopMachine(dev({ userAgent: IPAD, screenWidth: 1024, screenHeight: 1366, coarsePointer: true, anyFinePointer: false }))).toBe(false);
    expect(isDesktopMachine(dev({ userAgent: ANDROID_TABLET, screenWidth: 1848, screenHeight: 2960, maxTouchPoints: 10 }))).toBe(false);
    // A touch-first device with a mouse attached: the primary pointer is still touch.
    expect(isDesktopMachine(dev({ coarsePointer: true }))).toBe(false);
  });
});

describe("the mobile UI flag (#522)", () => {
  afterEach(() => vi.unstubAllGlobals());
  const memoryStorage = () => {
    const m = new Map<string, string>();
    return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
  };

  it("?mobile=1 asks for it and is remembered for the tab; ?mobile=0 forgets it", () => {
    vi.stubGlobal("sessionStorage", memoryStorage());
    expect(mobileUiRequested("")).toBe(false);
    expect(mobileUiRequested("?mobile=1")).toBe(true);
    // Later navigations (the project URL drops the query) keep it.
    expect(mobileUiRequested("")).toBe(true);
    expect(mobileUiRequested("?mobile=0")).toBe(false);
    expect(mobileUiRequested("")).toBe(false);
    expect(mobileUiRequested("?mobile")).toBe(true);
  });

  it("without storage, only the URL counts", () => {
    const denied = () => {
      throw new Error("denied");
    };
    vi.stubGlobal("sessionStorage", { getItem: denied, setItem: denied, removeItem: denied });
    expect(mobileUiRequested("?mobile=1")).toBe(true);
    expect(mobileUiRequested("?mobile=0")).toBe(false);
    expect(mobileUiRequested("")).toBe(false);
  });

  it("phones and touch-only tablets get it; ?force-desktop keeps the editor; ?mobile=1 asks for it anywhere", () => {
    vi.stubGlobal("sessionStorage", memoryStorage());
    const phone = dev({ userAgent: IPHONE, screenWidth: 390, screenHeight: 844, coarsePointer: true, anyFinePointer: false });
    const tablet = dev({ userAgent: ANDROID_TABLET, screenWidth: 1280, screenHeight: 800, coarsePointer: true, anyFinePointer: false });
    const ipadWithTrackpad = dev({ userAgent: IPAD, screenWidth: 1180, screenHeight: 820, coarsePointer: true, anyFinePointer: true, maxTouchPoints: 5 });
    expect(wantsMobileUi(phone, "")).toBe(true);
    expect(wantsMobileUi(tablet, "")).toBe(true);
    expect(wantsMobileUi(ipadWithTrackpad, "")).toBe(false);
    expect(wantsMobileUi(dev({}), "")).toBe(false);
    expect(wantsMobileUi(dev({}), "?mobile=1")).toBe(true);
    expect(wantsMobileUi(dev({}), "?mobile=0")).toBe(false);
    // Forced desktop, remembered for the tab.
    expect(wantsMobileUi(phone, "?force-desktop")).toBe(false);
    expect(wantsMobileUi(phone, "")).toBe(false);
    expect(wantsMobileUi(phone, "?mobile=1")).toBe(true);
  });

  it("asks for the playback audio session where there is one", () => {
    const nav = { audioSession: { type: "auto" } };
    preferPlaybackAudioSession(nav as unknown as Navigator);
    expect(nav.audioSession.type).toBe("playback");
    expect(() => preferPlaybackAudioSession({} as Navigator)).not.toThrow();
  });
});
