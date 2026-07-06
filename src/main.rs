mod alerts;
mod collectors;
mod config;
mod db;
mod metrics;
mod notify;
mod web;

use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::broadcast;

use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Arc::new(Config::load()?);

    let pool = db::init(&config.db_path).await?;
    let (tx, _rx) = broadcast::channel(256);
    let tx = Arc::new(tx);
    let notifier = Arc::new(notify::Notifier::new(&config));

    tokio::spawn(collectors::vps::run(
        pool.clone(),
        (*tx).clone(),
        notifier.clone(),
        config.clone(),
    ));
    tokio::spawn(collectors::docker::run(
        pool.clone(),
        (*tx).clone(),
        notifier.clone(),
        config.sample_interval(),
    ));
    tokio::spawn(purge_task(pool.clone(), config.retention_days));

    let web_docker = Arc::new(bollard::Docker::connect_with_local_defaults()?);
    let app = web::routes::router(tx, pool.clone(), web_docker);
    let listener = TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "rustmon listening");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn purge_task(pool: db::Pool, retention_days: i64) {
    let mut interval = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
    loop {
        interval.tick().await;
        if let Err(err) = db::purge_older_than(&pool, retention_days).await {
            tracing::warn!(%err, "failed to purge old metrics");
        }
    }
}
