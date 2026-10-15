//! Telemetry wire (observability; docs/OBSERVABILITY.md, docs/PRIVACY.md, CONTRACTS §20):
//! the JSON bodies the app posts to the `services/telemetry` Cloudflare Worker.
//!
//! Anonymous and opt-out: an install is a random UUID (resettable, never tied to an
//! account). Nothing here carries project content, file names, paths, user names, emails,
//! IP addresses or share room ids; clients scrub messages and stacks before sending and the
//! Worker scrubs them again.
//!
//! - `POST /v1/events` ← [`TelemetryBatch`] → [`TelemetryReply`]
//! - `POST /v1/crash` ← [`CrashReport`] → [`TelemetryReply`] (`fingerprint`)
//! - `POST /v1/diagnostics` ← [`DiagnosticsReport`] → [`TelemetryReply`] (`ticket`)
//!
//! Every request carries an `X-Ethereal-Version` header equal to `ctx.version`. Event names
//! and their props are an allowlist kept in the Worker (`services/telemetry/src/catalog.ts`):
//! unknown events and props are dropped there, so adding one is a two-line change
//! documented in docs/OBSERVABILITY.md.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Default endpoint: the Worker's workers.dev URL (`telemetry.ethereal.ws` once the ethereal.ws
/// zone is on the Cloudflare account). `ETHER_TELEMETRY_URL` overrides it; an
/// empty value disables telemetry.
pub const DEFAULT_TELEMETRY_URL: &str = "https://ethereal-telemetry.diegoimbert.workers.dev";
/// At most this many events per [`TelemetryBatch`].
pub const MAX_EVENTS_PER_BATCH: usize = 100;
/// Largest `/v1/events` body.
pub const MAX_EVENTS_BYTES: usize = 64 * 1024;
/// Largest `/v1/crash` body.
pub const MAX_CRASH_BYTES: usize = 256 * 1024;
/// Largest `/v1/diagnostics` body.
pub const MAX_DIAGNOSTICS_BYTES: usize = 1024 * 1024;

/// Which build sent a payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryPlatform {
    /// The Tauri desktop app.
    Desktop,
    /// The browser build (wasm engine).
    Web,
    /// `ether-server`, the headless remote engine.
    Server,
}

/// Who sent a payload. The same on every request of a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TelemetryContext {
    /// A random UUID v4 made on first run; "Reset install id" makes a new one.
    pub install_id: String,
    /// The app version (`VERSION`), e.g. `0.4.0`.
    pub version: String,
    /// `macos` | `windows` | `linux` | `ios` | `android` | `other`.
    pub os: String,
    /// `x86_64` | `aarch64` | `wasm32` | `other`.
    pub arch: String,
    pub platform: TelemetryPlatform,
}

/// One usage or health event. `name` and the keys of `props` must be in the Worker's
/// catalog; prop values are short, low-cardinality labels (`[a-z0-9_.+-]{1,32}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TelemetryEvent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub props: Option<BTreeMap<String, String>>,
    /// The event's one number (a duration in ms, a count); 1 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub value: Option<f64>,
}

/// `POST /v1/events`: at most [`MAX_EVENTS_PER_BATCH`] events, [`MAX_EVENTS_BYTES`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TelemetryBatch {
    pub ctx: TelemetryContext,
    pub events: Vec<TelemetryEvent>,
}

/// What crashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CrashKind {
    /// A Rust panic (any thread, including the engine's).
    Panic,
    /// An uncaught JS error (`window.onerror`).
    JsError,
    /// An unhandled promise rejection.
    UnhandledRejection,
    /// A wasm trap in the web engine.
    WasmTrap,
    /// A plugin sandbox process died.
    SandboxCrash,
}

/// The plugin a sandbox crash belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CrashPlugin {
    /// The plugin's id (bundle id / CLAP id / VST3 class id), not a path.
    pub id: String,
    /// `clap` | `vst3` | `vst2` | `au`.
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
}

/// `POST /v1/crash`: one report, at most [`MAX_CRASH_BYTES`]. Scrubbed by the sender and
/// again by the Worker, then grouped by a fingerprint of the message and top frames.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CrashReport {
    pub ctx: TelemetryContext,
    pub kind: CrashKind,
    /// The panic or error message (the Worker keeps 2 KiB).
    pub message: String,
    /// The backtrace or JS stack, one frame per line (the Worker keeps 64 KiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stack: Option<String>,
    /// The thread's name (`engine`, `main`, `ether-sandbox`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub plugin: Option<CrashPlugin>,
    /// A sandbox's exit code (or the signal number, negated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub exit_code: Option<i32>,
    /// Unix ms when it happened (it may be sent later, from the offline queue).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub at_ms: Option<u64>,
}

/// `POST /v1/diagnostics` (Help > Send diagnostics…): user-initiated, at most
/// [`MAX_DIAGNOSTICS_BYTES`]. Never a project file or audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DiagnosticsReport {
    pub ctx: TelemetryContext,
    /// What the user typed (optional, ≤ 4 KiB).
    #[serde(default)]
    pub description: String,
    /// System info: OS version, CPU count, memory, audio backend, sample rate, renderer...
    #[serde(default)]
    pub system: BTreeMap<String, String>,
    /// The tail of the engine log, scrubbed.
    #[serde(default)]
    pub log: String,
    /// A project summary: counts and plugin names, never the project itself.
    #[serde(default)]
    pub project: BTreeMap<String, String>,
}

/// The Worker's reply to every `POST`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TelemetryReply {
    pub ok: bool,
    /// Events written (`/v1/events`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub accepted: Option<u32>,
    /// Events dropped (unknown name, bad props).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dropped: Option<u32>,
    /// The crash group (`/v1/crash`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fingerprint: Option<String>,
    /// The ticket the user can quote (`/v1/diagnostics`), e.g. `D-7K3QX9`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ticket: Option<String>,
    /// Why a request was refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_report_round_trips_with_snake_case_tags() {
        let r = CrashReport {
            ctx: TelemetryContext {
                install_id: "8f0e0c3a-2a1b-4c4d-9e8f-0a1b2c3d4e5f".into(),
                version: "0.4.0".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
                platform: TelemetryPlatform::Desktop,
            },
            kind: CrashKind::SandboxCrash,
            message: "boom".into(),
            stack: None,
            thread: Some("engine".into()),
            plugin: Some(CrashPlugin {
                id: "com.x.y".into(),
                format: "clap".into(),
                version: None,
            }),
            exit_code: Some(-11),
            at_ms: Some(1),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"kind\":\"sandbox_crash\""));
        assert!(json.contains("\"platform\":\"desktop\""));
        assert!(!json.contains("stack"));
        assert_eq!(serde_json::from_str::<CrashReport>(&json).unwrap(), r);
    }
}
