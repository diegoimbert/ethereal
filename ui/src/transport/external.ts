// Opening a web link outside the app. One call for both hosts: `window.open` with `_blank`.
//
// - Desktop (Tauri): the main window's new-window handler (apps/desktop `create_main_window`)
//   denies the second webview and hands http(s)/mailto links to the system browser.
// - Web: a new tab; the editor tab keeps its project, its engine and its session.

/** Open `url` in the browser: the system browser on desktop, a new tab on web. */
export function openExternal(url: string): void {
  window.open(url, "_blank", "noopener,noreferrer");
}
