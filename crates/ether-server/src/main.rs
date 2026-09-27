//! `ether-server` binary: see `ether-server --help` and the crate docs.

use std::sync::Arc;

use ether_native::{DedicatedThread, HostConfig, HostOptions, NativeHost};
use ether_server::cli;
use ether_server::{Server, ServerConfig};

fn main() {
    if let Err(e) = run() {
        eprintln!("ether-server: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();

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
            ..HostOptions::default()
        },
    )
    .map_err(|e| e.to_string())?;
    tracing::info!(audio = ?host.audio_info(), projects_root = %projects_root.display(), "engine started");

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
    server.wait();
    Ok(())
}
