use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

pub type Pool = SqlitePool;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS vps_metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    cpu_usage REAL NOT NULL,
    ram_usage REAL NOT NULL,
    disk_usage REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS docker_metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    container_name TEXT NOT NULL,
    cpu_usage REAL NOT NULL,
    ram_usage REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS alerts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    component TEXT NOT NULL,
    message TEXT NOT NULL,
    status TEXT NOT NULL,
    acknowledged_at DATETIME,
    resolved_at DATETIME
);
"#;

/// Row of the `alerts` table. `status` is one of `TRIGGERED`, `ACKNOWLEDGED`, `RESOLVED`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Alert {
    pub id: i64,
    pub timestamp: String,
    pub component: String,
    pub message: String,
    pub status: String,
    pub acknowledged_at: Option<String>,
    pub resolved_at: Option<String>,
}

/// Opens (creating if absent) the SQLite file at `path` and applies the schema.
pub async fn init(path: &str) -> anyhow::Result<Pool> {
    let url = format!("sqlite://{path}?mode=rwc");
    let pool = SqlitePoolOptions::new().max_connections(5).connect(&url).await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    Ok(pool)
}

pub async fn insert_vps_metrics(
    pool: &Pool,
    cpu_usage: f64,
    ram_usage: f64,
    disk_usage: f64,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO vps_metrics (cpu_usage, ram_usage, disk_usage) VALUES (?, ?, ?)")
        .bind(cpu_usage)
        .bind(ram_usage)
        .bind(disk_usage)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_docker_metrics(
    pool: &Pool,
    container_name: &str,
    cpu_usage: f64,
    ram_usage: f64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO docker_metrics (container_name, cpu_usage, ram_usage) VALUES (?, ?, ?)",
    )
    .bind(container_name)
    .bind(cpu_usage)
    .bind(ram_usage)
    .execute(pool)
    .await?;
    Ok(())
}

/// Unix timestamps (seconds) of every sample recorded for `container_name`
/// within the last `window_hours` — used to build the uptime timeline.
pub async fn container_sample_timestamps(
    pool: &Pool,
    container_name: &str,
    window_hours: i64,
) -> anyhow::Result<Vec<i64>> {
    let cutoff = format!("-{window_hours} hours");
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT CAST(strftime('%s', timestamp) AS INTEGER) FROM docker_metrics \
         WHERE container_name = ?1 AND timestamp >= datetime('now', ?2)",
    )
    .bind(container_name)
    .bind(&cutoff)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(t,)| t).collect())
}

/// Inserts a new `TRIGGERED` alert, returning its id.
pub async fn insert_alert(pool: &Pool, component: &str, message: &str) -> anyhow::Result<i64> {
    let result =
        sqlx::query("INSERT INTO alerts (component, message, status) VALUES (?, ?, 'TRIGGERED')")
            .bind(component)
            .bind(message)
            .execute(pool)
            .await?;
    Ok(result.last_insert_rowid())
}

/// The `TRIGGERED` alert for `component`, if one is currently open. This is
/// the dedup gate: while it exists, no new notification is sent for the
/// same component.
pub async fn triggered_alert_for_component(
    pool: &Pool,
    component: &str,
) -> anyhow::Result<Option<Alert>> {
    let alert = sqlx::query_as::<_, Alert>(
        "SELECT * FROM alerts WHERE component = ? AND status = 'TRIGGERED' LIMIT 1",
    )
    .bind(component)
    .fetch_optional(pool)
    .await?;
    Ok(alert)
}

/// The `TRIGGERED` or `ACKNOWLEDGED` alert for `component`, if any — used to
/// decide whether an incident needs to be auto-resolved.
pub async fn open_alert_for_component(
    pool: &Pool,
    component: &str,
) -> anyhow::Result<Option<Alert>> {
    let alert = sqlx::query_as::<_, Alert>(
        "SELECT * FROM alerts WHERE component = ? AND status IN ('TRIGGERED', 'ACKNOWLEDGED') LIMIT 1",
    )
    .bind(component)
    .fetch_optional(pool)
    .await?;
    Ok(alert)
}

pub async fn acknowledge_alert(pool: &Pool, id: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE alerts SET status = 'ACKNOWLEDGED', acknowledged_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn resolve_alert(pool: &Pool, id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE alerts SET status = 'RESOLVED', resolved_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// All `TRIGGERED`/`ACKNOWLEDGED` alerts, newest first — drives the active
/// alerts panel.
pub async fn list_active_alerts(pool: &Pool) -> anyhow::Result<Vec<Alert>> {
    let alerts = sqlx::query_as::<_, Alert>(
        "SELECT * FROM alerts WHERE status IN ('TRIGGERED', 'ACKNOWLEDGED') ORDER BY timestamp DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(alerts)
}

/// Deletes samples older than `days` from both metrics tables.
pub async fn purge_older_than(pool: &Pool, days: i64) -> anyhow::Result<()> {
    let cutoff = format!("-{days} days");
    sqlx::query("DELETE FROM vps_metrics WHERE timestamp < datetime('now', ?)")
        .bind(&cutoff)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM docker_metrics WHERE timestamp < datetime('now', ?)")
        .bind(&cutoff)
        .execute(pool)
        .await?;
    Ok(())
}
