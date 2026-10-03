//! `ethereal://` deep links (docs/SHARING.md §5): invite links clicked in a browser, a chat
//! app or a mail client reach the UI's join screen.
//!
//! - `tauri-plugin-deep-link` registers the scheme (bundle config
//!   `plugins.deep-link.desktop.schemes`; macOS `CFBundleURLTypes`, the Linux `.desktop`
//!   MimeType and the Windows registry are written by the bundler). On Linux and Windows a
//!   release build also re-registers at start (AppImage, portable installs). Dev builds
//!   (`ETHER_INSTANCE`) never register, so a clicked link always goes to the installed app.
//! - `tauri-plugin-single-instance` (feature `deep-link`, release builds only, so several
//!   dev instances still run side by side) hands a second launch's URL to the running app.
//! - Every URL (cold start: `get_current()`; warm: `on_open_url`) goes into [`DeepLinkInbox`]
//!   and the shell emits [`DEEP_LINK_EVENT`] (no payload). The UI drains the inbox with the
//!   `take_deep_links` command when it starts and on each event, so a link that arrives
//!   before the webview listens is not lost and none is delivered twice.
//!
//! The UI decides what a link means (`ui/src/features/share/join/deepLink.ts`); the shell
//! only keeps `ethereal:` URLs of a sane length.

use std::collections::VecDeque;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Event emitted to the webview when a deep link arrived (drain with `take_deep_links`).
pub const DEEP_LINK_EVENT: &str = "ether://deep-link";
/// The URL scheme Ethereal registers.
pub const SCHEME: &str = "ethereal";
/// Longest URL kept (an invite is ≈ 85 characters, more with a custom signaling URL).
pub const MAX_URL_LEN: usize = 2048;
/// Links kept until the UI drains them (the oldest are dropped).
pub const MAX_PENDING: usize = 8;

/// Deep links received but not yet taken by the UI.
#[derive(Debug, Default)]
pub struct DeepLinkInbox(Mutex<VecDeque<String>>);

impl DeepLinkInbox {
    /// Queues `url` if it is an `ethereal:` URL of at most [`MAX_URL_LEN`] bytes. Returns
    /// whether it was kept.
    pub fn push(&self, url: &str) -> bool {
        let url = url.trim();
        if !is_ethereal_url(url) {
            return false;
        }
        let Ok(mut q) = self.0.lock() else {
            return false;
        };
        while q.len() >= MAX_PENDING {
            q.pop_front();
        }
        q.push_back(url.to_owned());
        true
    }

    /// Takes every pending link, oldest first.
    pub fn take(&self) -> Vec<String> {
        self.0
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }
}

/// `ethereal:...` (scheme case-insensitive), no control characters, ≤ [`MAX_URL_LEN`].
pub fn is_ethereal_url(url: &str) -> bool {
    url.len() <= MAX_URL_LEN
        && url
            .split_once(':')
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case(SCHEME))
        && !url.chars().any(char::is_control)
}

/// Queues `urls` and pings the webview when any was kept.
pub fn deliver<R: Runtime>(app: &AppHandle<R>, urls: impl IntoIterator<Item = String>) {
    let inbox = app.state::<DeepLinkInbox>();
    let mut kept = false;
    for url in urls {
        if inbox.push(&url) {
            kept = true;
        } else {
            tracing::debug!(len = url.len(), "ignored a non-ethereal deep link");
        }
    }
    if kept {
        if let Err(e) = app.emit(DEEP_LINK_EVENT, ()) {
            tracing::warn!(%e, "could not emit the deep-link event");
        }
    }
}

/// Plugin setup (call from `Builder::setup`): registers the scheme where needed, hooks
/// `on_open_url` and queues the link the app was started with.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    use tauri_plugin_deep_link::DeepLinkExt;

    #[cfg(any(target_os = "linux", windows))]
    if !cfg!(debug_assertions) {
        if let Err(e) = app.deep_link().register_all() {
            tracing::warn!(%e, "could not register the ethereal:// scheme");
        }
    }

    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        deliver(&handle, event.urls().into_iter().map(|u| u.to_string()));
    });
    match app.deep_link().get_current() {
        Ok(Some(urls)) => deliver(app, urls.into_iter().map(|u| u.to_string())),
        Ok(None) => {}
        Err(e) => tracing::debug!(%e, "no start-up deep link"),
    }
}

/// Brings the main window forward (a second launch, e.g. a clicked invite).
pub fn focus_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVITE: &str = "ethereal://join/AbCdEfGhIjKlMnOpQrStUv#10123456789_-abcdefghij";

    #[test]
    fn keeps_only_ethereal_urls() {
        assert!(is_ethereal_url(INVITE));
        assert!(is_ethereal_url("ETHEREAL://join/x"));
        assert!(is_ethereal_url("ethereal:join"));
        assert!(!is_ethereal_url("https://etherealws.pages.dev/join/x#1y"));
        assert!(!is_ethereal_url("etherealx://join/x"));
        assert!(!is_ethereal_url("ethereal"));
        assert!(!is_ethereal_url("ethereal://join/x\n#evil"));
        assert!(!is_ethereal_url(&format!(
            "ethereal://join/{}",
            "a".repeat(MAX_URL_LEN)
        )));
    }

    #[test]
    fn inbox_drains_in_order_once() {
        let inbox = DeepLinkInbox::default();
        assert!(inbox.push(&format!("  {INVITE}\n")));
        assert!(!inbox.push("file:///etc/passwd"));
        assert!(inbox.push("ethereal://join/second"));
        assert_eq!(inbox.take(), vec![INVITE.to_owned(), "ethereal://join/second".into()]);
        assert!(inbox.take().is_empty());
    }

    #[test]
    fn inbox_is_bounded() {
        let inbox = DeepLinkInbox::default();
        for i in 0..MAX_PENDING + 3 {
            inbox.push(&format!("ethereal://join/{i}"));
        }
        let all = inbox.take();
        assert_eq!(all.len(), MAX_PENDING);
        assert_eq!(all[0], "ethereal://join/3");
        assert_eq!(all[MAX_PENDING - 1], format!("ethereal://join/{}", MAX_PENDING + 2));
    }
}
