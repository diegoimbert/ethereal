/**
 * #622 (owner: "keep them permanently"): in the browser, projects (and their `versions/`
 * backups) live in OPFS, which the browser may clear under storage pressure unless the site's
 * storage is persistent. On launch the web app asks for it (`navigator.storage.persist()`):
 * Chrome and Edge decide by engagement (an installed app, a bookmark), Firefox asks the user,
 * Safari has its own rules. A refusal is retried once on the first click (engagement counts),
 * then said once, in a dismissible notice. Settings > General shows the state, the usage and
 * "request again". Every call is guarded: nothing here can stop the app from starting.
 */
import { t } from "@/i18n";
import { create } from "zustand";
import { notify } from "@/features/notifications";

export type Persistence = "unknown" | "persistent" | "best-effort" | "unsupported";

export interface StorageStatus {
  persistence: Persistence;
  /** Bytes used and available to the site (`navigator.storage.estimate()`), when known. */
  usage: number | null;
  quota: number | null;
}

export const useStorageStatus = create<StorageStatus>(() => ({ persistence: "unknown", usage: null, quota: null }));

/** The parts of `navigator.storage` this uses (injectable for tests). */
export type StorageApi = Partial<Pick<StorageManager, "persist" | "persisted" | "estimate">>;

const browserStorage = (): StorageApi | undefined => (typeof navigator === "undefined" ? undefined : navigator.storage);

/** localStorage: the refusal notice was shown (it is shown once). */
export const NOTICE_KEY = "eth.storage.persist-notice";

async function estimate(api: StorageApi): Promise<void> {
  try {
    const e = await api.estimate?.();
    if (e) useStorageStatus.setState({ usage: e.usage ?? null, quota: e.quota ?? null });
  } catch {
    // No estimate: the settings show the state only.
  }
}

/** Ask for persistent storage (unless it already is); returns the state reached. */
export async function requestPersistence(api: StorageApi | undefined = browserStorage()): Promise<Persistence> {
  let persistence: Persistence;
  if (!api?.persist) persistence = "unsupported";
  else {
    try {
      persistence = (await api.persisted?.()) || (await api.persist()) ? "persistent" : "best-effort";
    } catch {
      persistence = "best-effort";
    }
  }
  useStorageStatus.setState({ persistence });
  if (api) await estimate(api);
  return persistence;
}

/** Refresh the state and usage without asking (the settings section, when shown). */
export async function refreshStorageStatus(api: StorageApi | undefined = browserStorage()): Promise<void> {
  if (!api?.persist) {
    useStorageStatus.setState({ persistence: "unsupported" });
    return;
  }
  try {
    useStorageStatus.setState({ persistence: (await api.persisted?.()) ? "persistent" : "best-effort" });
  } catch {
    // Keep what is known.
  }
  await estimate(api);
}

function noticeShown(): boolean {
  try {
    return localStorage.getItem(NOTICE_KEY) === "1";
  } catch {
    return true;
  }
}

function showNoticeOnce(onOpenSettings?: () => void): void {
  if (noticeShown()) return;
  try {
    localStorage.setItem(NOTICE_KEY, "1");
  } catch {
    // Unavailable: `noticeShown` says it was shown, so it never repeats either.
  }
  notify("Warning", t("storage.refused_notice"), { title: t("storage.refused_title"), onClick: onOpenSettings });
}

/**
 * On launch (web): ask for persistent storage; if refused, ask again on the first click, and
 * if still refused, say so once. Never throws.
 */
export async function ensurePersistentStorage(
  opts: { api?: StorageApi; target?: Pick<Window, "addEventListener">; onOpenSettings?: () => void } = {},
): Promise<void> {
  const api = opts.api ?? browserStorage();
  const target = opts.target ?? (typeof window === "undefined" ? undefined : window);
  try {
    const first = await requestPersistence(api);
    if (first === "persistent") return;
    if (first === "unsupported" || !target) {
      showNoticeOnce(opts.onOpenSettings);
      return;
    }
    target.addEventListener(
      "pointerdown",
      () => {
        void requestPersistence(api).then((again) => {
          if (again !== "persistent") showNoticeOnce(opts.onOpenSettings);
        });
      },
      { once: true, capture: true },
    );
  } catch (e) {
    console.warn("ethereal: asking for persistent storage failed", e);
  }
}
