//! `ether-mcp` binary: see `ether-mcp --help`, docs/MCP.md and the crate docs.

use std::sync::Arc;

use ether_mcp::backend::Backend;
use ether_mcp::cli::{self, Mode};
use ether_mcp::headless::HeadlessBackend;
use ether_mcp::mcp::EtherMcp;
use ether_mcp::remote::{RemoteBackend, Target};
use rmcp::ServiceExt;

fn main() {
    // stdout is the MCP channel: logs go to stderr.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
    if let Err(e) = run() {
        eprintln!("ether-mcp: {e}");
        std::process::exit(1);
    }
}

fn backend(mode: Mode) -> Result<Arc<dyn Backend>, String> {
    Ok(match mode {
        Mode::Desktop { runtime_file } => {
            let candidates = match runtime_file {
                Some(f) => vec![f],
                None => ether_server::agent_bridge::desktop_runtime_candidates(
                    &ether_native::instance::instance_id(),
                ),
            };
            // Connected lazily: the app may be started (or the bridge enabled) later.
            Arc::new(RemoteBackend::new(Target::Desktop { candidates }))
        }
        Mode::Server { url, token } => {
            let b = RemoteBackend::new(Target::Fixed { url, token });
            b.connect()?;
            Arc::new(b)
        }
        Mode::Project { path } => {
            // ETHER_AUDIO=null semantics: the embedded engine never opens a device.
            Arc::new(HeadlessBackend::open(&path)?)
        }
    })
}

fn run() -> Result<(), String> {
    let args = cli::parse(std::env::args().skip(1), |k| std::env::var(k).ok())?;
    if args.help {
        println!("{}", cli::USAGE);
        return Ok(());
    }
    let backend = backend(args.mode)?;
    tracing::info!(engine = %backend.describe(), "ether-mcp starting");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let server = EtherMcp::new(backend);
    runtime.block_on(async move {
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| format!("MCP initialization failed: {e}"))?;
        service.waiting().await.map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })?;
    // Drop the engine (headless: stop it, remove the work dir) before exiting.
    drop(runtime);
    Ok(())
}
