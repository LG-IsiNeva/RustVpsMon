use std::time::Duration;

use serde::Deserialize;

/// Application configuration, loaded once at startup from `.env` / process
/// environment. All fields use the `RUSTMON_` prefix to avoid collisions
/// with unrelated env vars that may already exist on the host VPS.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_threshold_pct")]
    pub cpu_threshold_pct: f64,
    #[serde(default = "default_threshold_pct")]
    pub ram_threshold_pct: f64,
    #[serde(default = "default_threshold_pct")]
    pub disk_threshold_pct: f64,

    #[serde(default = "default_sample_interval_secs")]
    pub sample_interval_secs: u64,
    #[serde(default = "default_retention_days")]
    pub retention_days: i64,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    /// Mount point to scan for disk usage. Defaults to `/`; set to `/host`
    /// when running in Docker with the host root bind-mounted read-only at
    /// `/host`, otherwise disk metrics reflect the container's own overlay
    /// filesystem rather than the real VPS disk.
    #[serde(default = "default_disk_path")]
    pub disk_path: String,

    pub smtp_host: Option<String>,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    pub smtp_user: Option<String>,
    pub smtp_password: Option<String>,
    pub email_from: Option<String>,
    /// Raw comma-separated list of recipients. Kept as a plain `String`
    /// rather than `Vec<String>` since envy's delimited-list support for
    /// `Vec` fields is inconsistent across versions; parsed on demand via
    /// `email_to_list`.
    #[serde(default)]
    pub email_to: String,
    pub webhook_url: Option<String>,
}

fn default_threshold_pct() -> f64 {
    90.0
}

fn default_sample_interval_secs() -> u64 {
    5
}

fn default_retention_days() -> i64 {
    7
}

fn default_db_path() -> String {
    "data.db".to_string()
}

fn default_bind_addr() -> String {
    "0.0.0.0:3000".to_string()
}

fn default_disk_path() -> String {
    "/".to_string()
}

fn default_smtp_port() -> u16 {
    587
}

impl Config {
    /// Loads `.env` (a missing file is not an error, any other I/O/parse
    /// error is) then deserializes `RUSTMON_`-prefixed env vars into
    /// `Config`. Fails fast with context on malformed values.
    pub fn load() -> anyhow::Result<Self> {
        match dotenvy::dotenv() {
            Ok(_) => {}
            Err(dotenvy::Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        let mut config = envy::prefixed("RUSTMON_")
            .from_env::<Config>()
            .map_err(|err| anyhow::anyhow!("failed to load configuration: {err}"))?;

        // An env var set to an empty string (e.g. `RUSTMON_WEBHOOK_URL=` in
        // a docker-compose `environment:` block) deserializes to `Some("")`,
        // not `None` — normalize so "empty" reliably means "not configured".
        for field in [
            &mut config.smtp_host,
            &mut config.smtp_user,
            &mut config.smtp_password,
            &mut config.email_from,
            &mut config.webhook_url,
        ] {
            if field.as_deref() == Some("") {
                *field = None;
            }
        }

        Ok(config)
    }

    pub fn sample_interval(&self) -> Duration {
        Duration::from_secs(self.sample_interval_secs)
    }

    /// Parses the comma-separated `email_to` into a clean recipient list.
    pub fn email_to_list(&self) -> Vec<String> {
        self.email_to.split(',').map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect()
    }
}
