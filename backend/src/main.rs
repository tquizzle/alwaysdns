//! dnsui: a homelab DNS server (DoH upstreams, per-client policy) with a web UI API.
//!
//! Subcommands:
//!   dnsui serve                             run the DNS and HTTP servers (default)
//!   dnsui hash-password                     read a password on stdin, print an argon2 hash
//!   dnsui import-adguard <in> <out.yaml>    convert an AdGuardHome.yaml

mod adguard;
mod api;
mod app;
mod auth;
mod cache;
mod config;
mod db;
mod dns;
mod engine;
mod importer;
mod insights;
mod lists;
mod models;
mod recorder;
mod redact;
mod rules;
mod telemetry;
mod upstream;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hickory_server::server::Server;
use tokio::net::{TcpListener, UdpSocket};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::adguard::AdGuardSource;
use crate::app::{AppState, Options, StatsSource};
use crate::auth::Users;
use crate::dns::DnsHandler;

const USER_AGENT: &str = concat!("dnsui/", env!("CARGO_PKG_VERSION"));
const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const TCP_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
/// How often old query history and rollups are deleted.
const RETENTION_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[tokio::main]
async fn main() -> Result<()> {
    // One crypto backend for every TLS user (hickory DoH and reqwest).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        None | Some("serve") => serve().await,
        Some("hash-password") => hash_password_cmd(),
        Some("import-adguard") => import_cmd(&args[2..]),
        Some("vacuum") => vacuum_cmd(),
        Some("-h") | Some("--help") | Some("help") => {
            print_usage();
            Ok(())
        }
        Some(other) => {
            print_usage();
            bail!("unknown command '{other}'");
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage:\n  dnsui serve\n  dnsui hash-password   (password on stdin)\n  dnsui import-adguard <AdGuardHome.yaml> <out-config.yaml>\n  dnsui vacuum          (shrink dnsui.db; stop the server first)"
    );
}

/// One-off `VACUUM` of `dnsui.db` in `DNSUI_DATA_DIR`. Retention deletes rows but
/// SQLite keeps the freed pages for reuse, so the file never shrinks by itself.
fn vacuum_cmd() -> Result<()> {
    let data_dir = PathBuf::from(env_or("DNSUI_DATA_DIR", "data"));
    let path = data_dir.join("dnsui.db");
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }
    let (before, after) = db::vacuum(&path)?;
    let mb = |bytes: u64| bytes as f64 / 1_048_576.0;
    println!("{}: {:.1} MiB -> {:.1} MiB", path.display(), mb(before), mb(after));
    Ok(())
}

/// Shown once, on the first start, when the admin password was generated.
fn print_first_login(username: &str, password: &str, data_dir: &Path) {
    let rule = "=".repeat(64);
    println!();
    println!("{rule}");
    println!("  First start: admin account created");
    println!("    username: {username}");
    println!("    password: {password}");
    println!("  This password is shown once. Only its hash is stored in");
    println!("  {}/admin.json.", data_dir.display());
    println!("{rule}");
    println!();
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Boolean from the environment. Unset or empty gives `default`. Known false words (0, false, no, off) give false.
fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key).ok().map(|v| v.trim().to_ascii_lowercase()) {
        None => default,
        Some(v) if v.is_empty() => default,
        Some(v) => matches!(v.as_str(), "1" | "true" | "yes" | "on"),
    }
}

fn env_flag(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref().map(str::to_ascii_lowercase).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

fn build_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("building HTTP client")
}

fn stats_source_from_env(http: &reqwest::Client) -> Result<StatsSource> {
    match env_or("STATS_SOURCE", "native").to_ascii_lowercase().as_str() {
        "native" => Ok(StatsSource::Native),
        "adguard" => {
            let url = std::env::var("ADGUARD_URL").context("STATS_SOURCE=adguard needs ADGUARD_URL")?;
            let user = std::env::var("ADGUARD_USER").context("STATS_SOURCE=adguard needs ADGUARD_USER")?;
            let pass = std::env::var("ADGUARD_PASSWORD").context("STATS_SOURCE=adguard needs ADGUARD_PASSWORD")?;
            info!(%url, "stats source: AdGuard (read-only)");
            Ok(StatsSource::AdGuard(AdGuardSource::new(
                http.clone(),
                &url,
                &user,
                &pass,
            )?))
        }
        other => bail!("STATS_SOURCE must be 'native' or 'adguard', got '{other}'"),
    }
}

async fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let data_dir = PathBuf::from(env_or("DNSUI_DATA_DIR", "data"));
    // The config lives in the data directory so the Config page can save it. Docker's
    // data volume is writable; a bind-mounted single file is not replaceable.
    let config_path = PathBuf::from(env_or(
        "DNSUI_CONFIG",
        &data_dir.join("config.yaml").display().to_string(),
    ));
    let seed_path = PathBuf::from(env_or("DNSUI_SEED_CONFIG", "config.example.yaml"));
    if crate::config::Config::seed_if_missing(&config_path, &seed_path)? {
        info!(config = %config_path.display(), seed = %seed_path.display(), "created config from seed");
    }
    let opts = Options {
        config_path: config_path.clone(),
        cache_dir: PathBuf::from(env_or("DNSUI_CACHE_DIR", "cache")),
        static_dir: PathBuf::from(env_or("DNSUI_STATIC_DIR", "static")),
        cookie_secure: env_flag("DNSUI_COOKIE_SECURE"),
        // On by default. DNSUI_DB_ENABLED=false keeps the in-memory log and config lists.
        db_path: env_bool("DNSUI_DB_ENABLED", true).then(|| data_dir.join("dnsui.db")),
        // MaxMind GeoLite2-Country.mmdb. Provided by you (free MaxMind account); optional.
        geoip_db: Some(PathBuf::from(env_or(
            "DNSUI_GEOIP_DB",
            &data_dir.join("GeoLite2-Country.mmdb").display().to_string(),
        ))),
    };

    let admin = auth::resolve_admin(|key| std::env::var(key).ok(), &data_dir)?;
    if let Some(password) = &admin.generated_password {
        print_first_login(&admin.username, password, &data_dir);
    }
    let users = Users::from_env(admin)?;
    let http = build_http_client()?;
    let stats_source = stats_source_from_env(&http)?;

    info!(config = %config_path.display(), "loading configuration and lists");
    let state = AppState::new(http, opts, users, stats_source).await?;
    state.spawn_probes();
    let listen = state.runtime().config.listen.clone();

    // DNS over UDP and TCP.
    let mut dns = Server::new(DnsHandler::new(state.clone()));
    let udp = UdpSocket::bind(&listen.dns)
        .await
        .with_context(|| format!("binding UDP {}", listen.dns))?;
    dns.register_socket(udp);
    let tcp = TcpListener::bind(&listen.dns)
        .await
        .with_context(|| format!("binding TCP {}", listen.dns))?;
    dns.register_listener(tcp, TCP_IDLE_TIMEOUT, 4096);
    info!(addr = %listen.dns, "DNS listening (UDP and TCP)");
    tokio::spawn(async move {
        if let Err(e) = dns.block_until_done().await {
            error!(error = %e, "DNS server stopped");
        }
    });

    // Refresh lists every 24 hours.
    let refresher = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.tick().await; // the first tick fires immediately; lists were just loaded
        loop {
            ticker.tick().await;
            match refresher.refresh_lists().await {
                Ok(_) => info!("periodic list refresh complete"),
                Err(e) => warn!(error = %format!("{e:#}"), "periodic list refresh failed"),
            }
        }
    });

    // Delete query history and rollups past their retention window, hourly.
    let retainer = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(RETENTION_INTERVAL);
        loop {
            ticker.tick().await; // the first tick fires immediately
            match retainer.run_retention().await {
                Ok(Some((queries, minutes))) => {
                    if queries > 0 || minutes > 0 {
                        info!(queries, minutes, "retention pass deleted old rows");
                    }
                }
                Ok(None) => break, // database off: nothing to age out
                Err(e) => warn!(error = %format!("{e:#}"), "retention pass failed"),
            }
        }
    });

    // HTTP API and static UI.
    let app = api::router(state.clone());
    let http_listener = TcpListener::bind(&listen.http)
        .await
        .with_context(|| format!("binding HTTP {}", listen.http))?;
    info!(addr = %listen.http, static_dir = %state.static_dir.display(), "HTTP listening");
    axum::serve(http_listener, app)
        .with_graceful_shutdown(async {
            shutdown_signal().await;
            info!("shutting down");
        })
        .await
        .context("HTTP server failed")?;
    // Write the last queued query rows before the process exits.
    if !state.flush_query_log() {
        warn!("query log writer did not confirm the final flush");
    }
    Ok(())
}

/// Resolves on Ctrl-C or SIGTERM. Docker sends SIGTERM on `docker compose down`
/// and restart, so both must start the graceful shutdown and the final log flush.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

fn hash_password_cmd() -> Result<()> {
    // Read one line: Enter ends the input, so an interactive run does not wait for Ctrl-D.
    // Piped input works the same way (printf '%s\n' 'pw' | dnsui hash-password).
    eprint!("Admin password: ");
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .context("reading password from stdin")?;
    let password = input.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        bail!("no password entered (type it and press Enter)");
    }
    println!("{}", auth::hash_password(password)?);
    Ok(())
}

fn import_cmd(args: &[String]) -> Result<()> {
    let [input, output] = args else {
        print_usage();
        bail!("import-adguard needs an input file and an output file");
    };
    let input = Path::new(input);
    let output = Path::new(output);
    if output.exists() {
        bail!(
            "{} already exists; remove it or choose another path",
            output.display()
        );
    }

    let text = std::fs::read_to_string(input)
        .with_context(|| format!("reading {}", input.display()))?;
    let (config, report) = importer::import_adguard(&text)?;
    let yaml = config.to_yaml()?;
    std::fs::write(output, yaml).with_context(|| format!("writing {}", output.display()))?;

    println!("Wrote {}", output.display());
    println!("  clients:   {}", report.clients);
    println!("  lists:     {}", report.lists);
    println!("  allows:    {}", report.allows);
    println!("  blocks:    {}", report.blocks);
    println!("  rewrites:  {}", report.rewrites);
    println!("  dropped:   {}", report.dropped.len());
    println!("  changed:   {}", report.changed.len());

    if !report.dropped.is_empty() || !report.changed.is_empty() {
        eprintln!("\nImport report:");
        for line in &report.dropped {
            eprintln!("  DROPPED  {line}");
        }
        for line in &report.changed {
            eprintln!("  CHANGED  {line}");
        }
    }
    Ok(())
}
