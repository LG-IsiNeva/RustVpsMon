/// Snapshot of host resource usage, sampled by the VPS collector.
#[derive(Debug, Clone)]
pub struct VpsSample {
    pub cpu_usage: f64,
    pub ram_used: u64,
    pub ram_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
}

/// Container healthcheck status, derived from Docker's `Health.Status`.
/// `Unknown` covers containers with no `HEALTHCHECK` defined, in a
/// `starting` state, or reported by a daemon too old to expose health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Unknown,
}

impl HealthStatus {
    pub fn pill_class(&self) -> &'static str {
        match self {
            Self::Healthy => "pill-ok",
            Self::Unhealthy => "pill-warn",
            Self::Unknown => "pill-neutral",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Healthy => "HEALTHY",
            Self::Unhealthy => "UNHEALTHY",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Snapshot of a single container's resource usage, sampled by the Docker collector.
#[derive(Debug, Clone)]
pub struct DockerSample {
    pub container_name: String,
    /// Image repository, without tag (e.g. `traefik`).
    pub image_repo: String,
    /// Image tag/version (e.g. `v3.7.6`), empty if untagged.
    pub version: String,
    pub is_running: bool,
    pub health: HealthStatus,
    /// Human relative time since the container was created (e.g. `9h ago`).
    pub created_ago: String,
    /// Human relative time since the container was last started (e.g. `9h ago`),
    /// distinct from `created_ago` when the container has been restarted.
    /// `—` if the daemon never reported a start time.
    pub started_ago: String,
    /// Fixed-length uptime timeline over the last 24h, oldest first. Each
    /// entry is a CSS class: `bar-na` (container didn't exist yet),
    /// `bar-up` (sampled = alive that bucket), `bar-down` (no sample =
    /// presumed down), with the last bucket suffixed `bar-live` to reflect
    /// current status regardless of sampling history.
    pub timeline: Vec<&'static str>,
}

/// Broadcast payload: every collector feeds samples into one shared channel,
/// the SSE route fans them back out as rendered HTML fragments.
#[derive(Debug, Clone)]
pub enum MetricEvent {
    Vps(VpsSample),
    /// Full snapshot of all containers sampled in one collector cycle —
    /// sent as a single event so the SSE fragment replaces the whole list
    /// instead of one container overwriting another on `sse-swap`.
    Docker(Vec<DockerSample>),
    /// Full snapshot of currently active (triggered/acknowledged) alerts.
    Alerts(Vec<crate::db::Alert>),
}
