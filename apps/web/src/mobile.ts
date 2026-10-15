// Phones and touch-only tablets get the mobile UI (#522): the touch shell instead of the desktop
// editor. The rule (`isMobileDevice`), on the physical device, not the window, so a small
// desktop window keeps the desktop editor:
//   - no fine pointer anywhere (`any-pointer: fine` is false: no mouse, trackpad or pen), and
//   - the primary pointer is coarse (touch), or the screen's short side is under 600 CSS px
//     (`min(screen.width, screen.height)`: phones, in either orientation, even when the browser
//     misreports its pointers or asks for the desktop site).
// An Android / iPhone / iPod user agent also counts when no fine pointer is attached.
// Tablets with a keyboard or trackpad (iPad + Magic Keyboard) report a fine pointer and keep the
// desktop editor; touch-only tablets get the mobile UI in its tablet layout. Laptops with a
// touchscreen keep a fine primary pointer and the desktop editor.
//
// Overrides, remembered for the tab: `?force-desktop` gives the desktop editor anywhere,
// `?mobile=1` the mobile UI anywhere (testing on a desktop; `?mobile=0` forgets it).

/** Phones are under this on their short side (CSS px of `screen`); a Fold unfolded is above it. */
export const PHONE_SHORT_SIDE_PX = 600;
export const FORCE_DESKTOP_PARAM = "force-desktop";
const FORCE_DESKTOP_KEY = "eth.forceDesktop";
export const MOBILE_UI_PARAM = "mobile";
const MOBILE_UI_KEY = "eth.mobileUi";

export interface DeviceInfo {
  userAgent: string;
  screenWidth: number;
  screenHeight: number;
  /** `matchMedia("(pointer: coarse)")`: the primary pointer is touch. */
  coarsePointer: boolean;
  /** `matchMedia("(any-pointer: fine)")`: some mouse / trackpad / pen is available. */
  anyFinePointer: boolean;
  /** `navigator.maxTouchPoints` (0 on a Mac; an iPad asking for the desktop site reports more). */
  maxTouchPoints?: number;
}

const PHONE_UA = /iPhone|iPod|Android.*Mobile|Windows Phone/i;

export function isMobileDevice(d: DeviceInfo): boolean {
  if (d.anyFinePointer) return false;
  const shortSide = Math.min(d.screenWidth, d.screenHeight);
  const small = shortSide > 0 && shortSide < PHONE_SHORT_SIDE_PX;
  return d.coarsePointer || small || PHONE_UA.test(d.userAgent);
}

const TABLET_UA = /iPad|Android|Silk|Kindle|PlayBook|Tablet/i;

/**
 * A desktop or laptop computer, the kind the desktop app runs on (the "Open on desktop" band):
 * not a phone (`isMobileDevice`), its primary pointer is fine (a mouse or trackpad), and not a
 * tablet, even one with a keyboard and trackpad (an Android or iPad user agent, or a "Macintosh"
 * with a touchscreen: iPadOS asking for the desktop site).
 */
export function isDesktopMachine(d: DeviceInfo): boolean {
  if (isMobileDevice(d) || d.coarsePointer || TABLET_UA.test(d.userAgent)) return false;
  return !(/Macintosh/i.test(d.userAgent) && (d.maxTouchPoints ?? 0) > 1);
}

export function currentDevice(): DeviceInfo {
  const mq = (q: string) => typeof matchMedia === "function" && matchMedia(q).matches;
  return {
    userAgent: navigator.userAgent,
    screenWidth: screen.width,
    screenHeight: screen.height,
    coarsePointer: mq("(pointer: coarse)"),
    anyFinePointer: mq("(any-pointer: fine)"),
    maxTouchPoints: navigator.maxTouchPoints ?? 0,
  };
}

/** `?force-desktop` in the URL, or remembered from earlier in this tab. */
export function forcedDesktop(search: string = location.search): boolean {
  try {
    if (new URLSearchParams(search).has(FORCE_DESKTOP_PARAM)) {
      sessionStorage.setItem(FORCE_DESKTOP_KEY, "1");
      return true;
    }
    return sessionStorage.getItem(FORCE_DESKTOP_KEY) === "1";
  } catch {
    return new URLSearchParams(search).has(FORCE_DESKTOP_PARAM);
  }
}

/**
 * The mobile UI was asked for: `?mobile=1` in the URL, or earlier in this tab (`?mobile=0`
 * forgets it).
 */
export function mobileUiRequested(search: string = location.search): boolean {
  const param = new URLSearchParams(search).get(MOBILE_UI_PARAM);
  try {
    if (param === "0") sessionStorage.removeItem(MOBILE_UI_KEY);
    else if (param !== null) sessionStorage.setItem(MOBILE_UI_KEY, "1");
    return sessionStorage.getItem(MOBILE_UI_KEY) === "1";
  } catch {
    return param !== null && param !== "0";
  }
}

/**
 * Lets the system treat the app's audio as music (Safari 16.4+ `navigator.audioSession`): it
 * plays with the ringer switch on silent, like any music app.
 */
export function preferPlaybackAudioSession(nav: Navigator = navigator): void {
  const session = (nav as Navigator & { audioSession?: { type: string } }).audioSession;
  try {
    if (session) session.type = "playback";
  } catch {
    // read-only or unsupported: the default session
  }
}

/**
 * The mobile UI for this page: asked for (`?mobile=1`), or a phone or touch-only tablet that
 * did not force the desktop editor.
 */
export function wantsMobileUi(device: DeviceInfo = currentDevice(), search: string = location.search): boolean {
  if (mobileUiRequested(search)) return true;
  return !forcedDesktop(search) && isMobileDevice(device);
}

/** The download page (`/download`, `/download/`): standalone, phones included. */
export const isDownloadPage = (pathname: string): boolean => /^\/download\/?$/.test(pathname);
