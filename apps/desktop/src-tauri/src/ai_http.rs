//! `ai-chat` on the desktop: model provider requests go through `tauri-plugin-http` (fetched
//! in Rust, so CORS doesn't apply) instead of the webview's `fetch`.
//!
//! The plugin only fetches URLs its scope allows:
//! - `capabilities/ai-providers.json` allows the preset providers' default base URLs;
//! - [`ai_http_allow`] adds the origin of a base URL the user configured (a changed preset
//!   or a custom OpenAI-compatible server) as a runtime capability. The URL is validated
//!   first ([`endpoint_origin`]): http(s) only, no credentials, a plain host, so the scope
//!   pattern built from it can't match anything wider than that one origin.
//!
//! Every other URL is refused by the plugin ("url not allowed on the configured scope").
//! Requests carry the user's API key: nothing here logs a URL's path or query, or a header.

use std::collections::BTreeSet;
use std::sync::Mutex;

use tauri::ipc::CapabilityBuilder;
use tauri::{AppHandle, Manager, Runtime, State, Url};

/// Runtime origins at most (a user switches between a handful of servers).
const MAX_ORIGINS: usize = 64;

/// The origins added at runtime so far (each once).
#[derive(Default)]
pub struct AiHttpScope(Mutex<BTreeSet<String>>);

/// The origin (`scheme://host[:port]`) of a provider base URL, or why it can't be allowed.
pub fn endpoint_origin(base_url: &str) -> Result<String, String> {
    let url =
        Url::parse(base_url.trim()).map_err(|_| "the base URL isn't a valid URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("the base URL must start with http:// or https://".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("the base URL can't contain a user name or password".into());
    }
    let Some(host) = url.host_str() else {
        return Err("the base URL has no host".into());
    };
    // `[::1]` would need escaping in a URL pattern; local servers answer on localhost.
    if host.starts_with('[') {
        return Err("IPv6 addresses aren't supported here: use a host name".into());
    }
    // The host goes into a URL pattern: only plain DNS characters (an IPv4 address is
    // digits and dots), so it can't widen the pattern (`*`, `(`, `{`, `:` ... are syntax).
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
    {
        return Err("the base URL's host isn't a valid host name".into());
    }
    Ok(url.origin().ascii_serialization())
}

/// The plugin scope pattern for an origin from [`endpoint_origin`].
pub fn scope_pattern(origin: &str) -> String {
    format!("{origin}/*")
}

/// Allows the AI chat to fetch `base_url`'s origin through the HTTP plugin (until the app
/// quits). Called by the UI before each provider request; adding an origin twice is a no-op.
#[tauri::command]
pub fn ai_http_allow<R: Runtime>(
    app: AppHandle<R>,
    scope: State<'_, AiHttpScope>,
    base_url: String,
) -> Result<(), String> {
    let origin = endpoint_origin(&base_url)?;
    let mut origins = scope
        .0
        .lock()
        .map_err(|_| "AI scope unavailable".to_string())?;
    if origins.contains(&origin) {
        return Ok(());
    }
    if origins.len() >= MAX_ORIGINS {
        return Err("too many AI provider addresses this session: restart the app".into());
    }
    let capability = CapabilityBuilder::new(format!("ai-endpoint-{}", origins.len()))
        .window("main")
        .permission_scoped(
            "http:default",
            vec![serde_json::json!({ "url": scope_pattern(&origin) })],
            Vec::<serde_json::Value>::new(),
        );
    app.add_capability(capability)
        .map_err(|e| format!("could not allow {origin}: {e}"))?;
    tracing::info!(%origin, "ai chat: provider origin allowed");
    origins.insert(origin);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewWindow, WebviewWindowBuilder};

    #[test]
    fn origins_are_validated() {
        assert_eq!(
            endpoint_origin("https://api.example.com/v1/").unwrap(),
            "https://api.example.com"
        );
        assert_eq!(
            endpoint_origin(" http://localhost:8080/v1 ").unwrap(),
            "http://localhost:8080"
        );
        assert_eq!(
            endpoint_origin("http://192.168.1.20:8000").unwrap(),
            "http://192.168.1.20:8000"
        );
        for bad in [
            "",
            "not a url",
            "file:///etc/passwd",
            "ftp://example.com",
            "https://user:pass@example.com",
            "https://*.example.com/v1",
            "https://exa(mple).com",
            "http://[::1]:8080",
        ] {
            assert!(endpoint_origin(bad).is_err(), "{bad} should be refused");
        }
    }

    fn app() -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>) {
        let app = mock_builder()
            .plugin(tauri_plugin_http::init())
            .manage(AiHttpScope::default())
            .invoke_handler(tauri::generate_handler![ai_http_allow])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let window = match app.get_webview_window("main") {
            Some(w) => w,
            None => WebviewWindowBuilder::new(&app, "main", Default::default())
                .build()
                .expect("main window"),
        };
        (app, window)
    }

    fn invoke(
        window: &WebviewWindow<MockRuntime>,
        cmd: &str,
        body: serde_json::Value,
    ) -> Result<(), String> {
        get_ipc_response(
            window,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: if cfg!(windows) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// `plugin:http|fetch` only creates the request (sending is `fetch_send`), so an allowed
    /// URL succeeds offline and a refused one fails on the scope.
    fn fetch(window: &WebviewWindow<MockRuntime>, url: &str) -> Result<(), String> {
        invoke(
            window,
            "plugin:http|fetch",
            serde_json::json!({ "clientConfig": { "method": "POST", "url": url, "headers": [], "data": null } }),
        )
    }

    #[test]
    fn plugin_scope_allows_providers_and_refuses_other_hosts() {
        let (_app, w) = app();
        // Preset providers (capabilities/ai-providers.json).
        for url in [
            "https://api.anthropic.com/v1/messages",
            "https://api.openai.com/v1/chat/completions",
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
            "http://localhost:11434/v1/models",
        ] {
            assert_eq!(fetch(&w, url), Ok(()), "{url}");
        }
        // Anything else is refused.
        for url in [
            "https://example.com/",
            "https://api.openai.com.evil.example/v1",
            "http://api.openai.com/v1/chat/completions",
            "http://localhost:9999/v1/models",
        ] {
            let err = fetch(&w, url).unwrap_err();
            assert!(err.contains("not allowed"), "{url}: {err}");
        }

        // A custom endpoint becomes reachable once allowed, and nothing else with it.
        assert!(fetch(&w, "http://localhost:8080/v1/chat/completions").is_err());
        invoke(
            &w,
            "ai_http_allow",
            serde_json::json!({ "baseUrl": "http://localhost:8080/v1" }),
        )
        .unwrap();
        assert_eq!(
            fetch(&w, "http://localhost:8080/v1/chat/completions"),
            Ok(())
        );
        assert!(fetch(&w, "http://localhost:8081/v1/chat/completions").is_err());
        // Twice is fine.
        invoke(
            &w,
            "ai_http_allow",
            serde_json::json!({ "baseUrl": "http://localhost:8080/v1" }),
        )
        .unwrap();
        // A wildcard host is refused before it reaches the scope.
        let err = invoke(
            &w,
            "ai_http_allow",
            serde_json::json!({ "baseUrl": "https://*.example.com" }),
        )
        .unwrap_err();
        assert!(err.contains("host"), "{err}");
        assert!(fetch(&w, "https://a.example.com/").is_err());
    }
}
