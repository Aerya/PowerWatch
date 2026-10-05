mod app;
mod state;
mod suggestions;

use clap::Parser;
use powerwatch_core::runtime::build_default_sampler;
use powerwatch_core::storage::Storage;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Parser)]
struct Args {
    /// Address to listen on. Keep the default for local-only access.
    /// Docker may use 0.0.0.0 internally, but publish the port only on a trusted LAN IP.
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,

    #[arg(long, default_value_t = 3000)]
    port: u16,

    /// Persist history to SQLite.
    #[arg(long)]
    log: bool,

    /// Seconds between persisted history samples. Live monitoring still refreshes every second.
    #[arg(long, default_value_t = 60)]
    history_interval: u64,

    /// NAS/headless mode: disable desktop-oriented energy suggestions and actions.
    #[arg(long)]
    nas_mode: bool,
}

fn history_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".local/share/powerwatch/history.db")
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let sampler = build_default_sampler();
    if sampler.is_empty() {
        eprintln!("no power sensors available on this machine - nothing to show");
        std::process::exit(1);
    }

    let db_path = history_db_path();
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let storage = Storage::open(&db_path).ok();
    if args.log && storage.is_none() {
        eprintln!(
            "warning: --log was passed but the history database at {} couldn't be opened",
            db_path.display()
        );
    }

    let history_interval = Duration::from_secs(args.history_interval.max(1));
    let app_state = state::start_sampling_loop(
        sampler,
        SAMPLE_INTERVAL,
        storage,
        args.log,
        history_interval,
        !args.nas_mode,
    );

    let router = app::build_router(app_state);
    let addr = (args.host, args.port);

    if !args.host.is_loopback() {
        eprintln!("WARNING: PowerWatch has no authentication.");
        eprintln!("Expose this listener only on a trusted LAN.");
        eprintln!("Do NOT publish it through a public reverse proxy or the Internet.");
    }

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("failed to bind {}:{}: {e}", args.host, args.port);
            std::process::exit(1);
        }
    };

    println!(
        "powerwatch-web listening on http://{}:{}",
        args.host, args.port
    );
    if args.log {
        println!(
            "logging history every {}s to {}",
            args.history_interval.max(1),
            db_path.display()
        );
    }
    if args.nas_mode {
        println!("NAS mode enabled: desktop energy suggestions are disabled");
    }

    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}
