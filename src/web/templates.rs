use askama::Template;

use crate::db::Alert;
use crate::metrics::{DockerSample, VpsSample};

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate;

/// CSS color keyword driving both the bar fill and the percent text,
/// matching the ok/warn/critical status thresholds.
fn status_color(pct: f64) -> &'static str {
    if pct >= 90.0 {
        "var(--mnt-status-critical)"
    } else if pct >= 70.0 {
        "var(--mnt-status-warn)"
    } else {
        "var(--mnt-status-ok)"
    }
}

#[derive(Template)]
#[template(path = "vps_metrics.html")]
pub struct VpsMetricsTemplate {
    pub cpu_pct: u32,
    pub cpu_color: &'static str,
    pub mem_pct: u32,
    pub mem_color: &'static str,
    pub disk_pct: u32,
    pub disk_color: &'static str,
}

impl From<VpsSample> for VpsMetricsTemplate {
    fn from(s: VpsSample) -> Self {
        let cpu_pct = s.cpu_usage.round() as u32;
        let mem_pct = if s.ram_total > 0 {
            (s.ram_used as f64 / s.ram_total as f64 * 100.0).round() as u32
        } else {
            0
        };
        let disk_pct = if s.disk_total > 0 {
            (s.disk_used as f64 / s.disk_total as f64 * 100.0).round() as u32
        } else {
            0
        };

        Self {
            cpu_pct,
            cpu_color: status_color(cpu_pct as f64),
            mem_pct,
            mem_color: status_color(mem_pct as f64),
            disk_pct,
            disk_color: status_color(disk_pct as f64),
        }
    }
}

#[derive(Template)]
#[template(path = "docker_metrics.html")]
pub struct DockerMetricsTemplate {
    pub containers: Vec<DockerSample>,
}

impl From<Vec<DockerSample>> for DockerMetricsTemplate {
    fn from(containers: Vec<DockerSample>) -> Self {
        Self { containers }
    }
}

#[derive(Template)]
#[template(path = "container_logs.html")]
pub struct ContainerLogsTemplate {
    pub logs: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "alerts.html")]
pub struct AlertsTemplate {
    pub alerts: Vec<Alert>,
    pub unacknowledged_count: usize,
}

impl From<Vec<Alert>> for AlertsTemplate {
    fn from(alerts: Vec<Alert>) -> Self {
        let unacknowledged_count = alerts.iter().filter(|a| a.status == "TRIGGERED").count();
        Self { alerts, unacknowledged_count }
    }
}
