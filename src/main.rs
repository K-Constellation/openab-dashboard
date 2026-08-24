mod config;
mod db;
mod models;
mod server;
mod collector;
mod pod_matching;

pub(crate) use pod_matching::is_deployment_pod_name;

use clap::Parser;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "openab-dashboard", about = "AI tool usage monitoring dashboard")]
struct Cli {
    /// Path to config.yaml
    #[arg(short, long, default_value = "config.yaml")]
    config: String,

    /// Port for web server
    #[arg(short, long, default_value_t = 8080)]
    port: u16,

    /// Run collector once and exit (for testing)
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("openab_dashboard=info".parse()?))
        .init();

    let cli = Cli::parse();
    tracing::info!("loading config from {}", cli.config);

    let config = config::load(&cli.config)?;
    let db = db::Database::new(&config.database.path)?;
    db.initialize()?;

    let state = Arc::new(AppState {
        db,
        config: config.clone(),
    });

    if cli.once {
        tracing::info!("running single collection");
        collector::collect_once(state.clone()).await?;
        tracing::info!("collection complete");
        return Ok(());
    }

    // Start collector in background
    let collector_state = state.clone();
    tokio::spawn(async move {
        collector::run_scheduler(collector_state).await;
    });

    // Start web server
    tracing::info!("starting web server on port {}", cli.port);
    server::start(state, cli.port).await?;

    Ok(())
}

pub struct AppState {
    pub db: db::Database,
    pub config: config::Config,
}
