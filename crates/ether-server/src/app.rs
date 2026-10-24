//! The `ether-server` process: what its binary runs, with an extension's [`Hooks`]
//! (docs/EXTENSIONS.md): a product with extensions builds an `ether-server` binary of its own
//! that calls [`main`] with them.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ether_native::{DedicatedThread, HostConfig, HostOptions, NativeHost};

use crate::cli;
use crate::{Server, ServerConfig};

/// What runs once the process is shutting down (after the server and engine stopped).
pub type Stopped = Box<dyn FnOnce()>;

/// An extension's part of the process.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// First thing in the process (after logging): the extension's process-wide points
    /// (controller factory or setup, bridge extension, plugin formats).
    pub startup: fn(),
    /// Once the data dir exists (`dev`: a debug build), before the engine starts: what the
    /// extension runs for the process; the returned closure runs at shutdown.
    pub started: fn(data_dir: &Path, dev: bool) -> Option<Stopped>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            startup: || {},
            started: |_, _| None,
        }
    }
}

/// The process: parse the command line, start the engine and the server, serve until
/// SIGTERM/SIGINT; on error print it and exit 1.
pub fn main(hooks: Hooks) {
    if let Err(e) = run(&hooks) {
        eprintln!("ether-server: {e}");
        std::process::exit(1);
    }
}

fn run(hooks: &Hooks) -> Result<(), String> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
    (hooks.startup)();

    let args = cli::parse(std::env::args().skip(1))?;
    if args.help {
        println!("{}", cli::USAGE);
        return Ok(());
    }
    let dev = cfg!(debug_assertions);
    let instance = ether_native::instance::instance_id();
    let data_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(|| cli::default_data_dir(dev, &instance));
    std::fs::create_dir_all(data_dir.join("config")).map_err(|e| e.to_string())?;
    let stopped = (hooks.started)(&data_dir, dev);
    let projects_root = args
        .projects
        .clone()
        .unwrap_or_else(|| ether_native::default_projects_root(dev, &data_dir));
    std::fs::create_dir_all(&projects_root).map_err(|e| e.to_string())?;

    let port = cli::port(&args, |k| std::env::var(k).ok())?;
    let bind = cli::bind(&args, port);
    let mut generated = false;
    let token = if args.no_auth {
        None
    } else if let Some(t) = args
        .token
        .clone()
        .or_else(|| std::env::var("ETHER_SERVER_TOKEN").ok())
        .filter(|t| !t.is_empty())
    {
        Some(t)
    } else {
        generated = true;
        Some(cli::load_or_create_token(&data_dir).map_err(|e| format!("token file: {e}"))?)
    };

    let host = NativeHost::start(
        HostConfig {
            audio: None,
            data_dir: data_dir.clone(),
            instance: instance.clone(),
            projects_root: projects_root.clone(),
            library_roots: cli::library_roots(&args, &data_dir),
        },
        HostOptions {
            main_thread: Arc::new(DedicatedThread::new()),
            // Remote clients name engine-machine files only when asked (`Unsupported`
            // otherwise, as the protocol documents for hosts whose UI is elsewhere).
            os_paths: args.allow_os_paths,
            ..HostOptions::default()
        },
    )
    .map_err(|e| e.to_string())?;
    tracing::info!(audio = ?host.audio_info(), projects_root = %projects_root.display(), "engine started");

    // SIGTERM/SIGINT (service stop, Ctrl+C): close the clients and stop the engine, which
    // saves a dirty project. A second signal exits at once. Registered before the server
    // starts, so no signal can skip the save once clients may have edited.
    let term = Arc::new(AtomicBool::new(false));
    for &sig in signal_hook::consts::TERM_SIGNALS {
        signal_hook::flag::register_conditional_shutdown(sig, 1, term.clone())
            .and_then(|_| signal_hook::flag::register(sig, term.clone()))
            .map_err(|e| format!("signal handler: {e}"))?;
    }

    let server = Server::start(
        ServerConfig {
            bind,
            token: token.clone(),
            name: args.name.clone(),
            allow_multiple_clients: !args.single_client,
            instance,
            projects_root: Some(projects_root),
            ..ServerConfig::default()
        },
        host,
    )
    .map_err(|e| e.to_string())?;

    // Machine-readable first line (scripts and e2e wait for it).
    println!("ether-server listening on ws://{}/", server.local_addr());
    match (&token, args.print_token) {
        (None, _) => println!("auth: disabled (loopback only)"),
        (Some(t), true) => println!("token: {t}"),
        (Some(_), false) if generated => println!(
            "token: see {} (or run with --print-token)",
            cli::token_file(&data_dir).display()
        ),
        (Some(_), false) => {}
    }
    // The app hides the engine server form until asked for.
    println!(
        "connect: in Ethereal, run \"engine server…\" from the command palette (or open the web app with ?engine-server)"
    );
    while !term.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    tracing::info!("shutting down");
    server.shutdown();
    tracing::info!("stopped");
    if let Some(stopped) = stopped {
        stopped();
    }
    Ok(())
}
