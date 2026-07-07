use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bollard::Docker;
use bollard::models::{ContainerSummaryHealth, ContainerSummaryHealthStatusEnum, ContainerSummaryStateEnum};
use bollard::query_parameters::{ListContainersOptions, StatsOptions};
use futures_util::StreamExt;
use tokio::sync::broadcast;

use crate::alerts;
use crate::db::{self, Pool};
use crate::metrics::{DockerSample, HealthStatus, MetricEvent};
use crate::notify::Notifier;

/// Maps Docker's health status to our badge state. Containers with no
/// `HEALTHCHECK` defined, still `starting`, or on a daemon too old to
/// report health all collapse to `Unknown`.
fn map_health(health: Option<ContainerSummaryHealth>) -> HealthStatus {
    match health.and_then(|h| h.status) {
        Some(ContainerSummaryHealthStatusEnum::HEALTHY) => HealthStatus::Healthy,
        Some(ContainerSummaryHealthStatusEnum::UNHEALTHY) => HealthStatus::Unhealthy,
        _ => HealthStatus::Unknown,
    }
}

const TIMELINE_BUCKETS: usize = 48;
const TIMELINE_WINDOW_HOURS: i64 = 24;
const TIMELINE_WINDOW_SECS: i64 = TIMELINE_WINDOW_HOURS * 3600;

/// Discovers running containers and samples their CPU/RAM every `interval`.
/// If the Docker socket is unreachable, logs once and idles rather than
/// crashing the process — VPS metrics keep flowing regardless.
pub async fn run(
    pool: Pool,
    tx: broadcast::Sender<MetricEvent>,
    notifier: Arc<Notifier>,
    interval: Duration,
) {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(err) => {
            tracing::warn!(%err, "docker socket unavailable, container monitoring disabled");
            return;
        }
    };

    loop {
        let list_options =
            ListContainersOptions { all: true, limit: None, size: false, filters: None };
        match docker.list_containers(Some(list_options)).await {
            Ok(containers) => {
                let mut samples = Vec::with_capacity(containers.len());

                for container in containers {
                    let Some(id) = container.id else { continue };
                    let name = container
                        .names
                        .and_then(|n| n.into_iter().next())
                        .unwrap_or_else(|| id.clone())
                        .trim_start_matches('/')
                        .to_string();

                    let is_running = container.state == Some(ContainerSummaryStateEnum::RUNNING);
                    let health = map_health(container.health);
                    let created_at = container.created.unwrap_or(0);
                    let (image_repo, version) = split_image(container.image.as_deref().unwrap_or(""));

                    alerts::evaluate(
                        &pool,
                        &tx,
                        &notifier,
                        &format!("docker_container_{name}"),
                        format!("Container {name} is down"),
                        !is_running,
                    )
                    .await;

                    if let Some((cpu_usage, ram_usage)) = sample_stats(&docker, &id).await {
                        if let Err(err) =
                            db::insert_docker_metrics(&pool, &name, cpu_usage, ram_usage).await
                        {
                            tracing::warn!(%err, "failed to persist docker metrics");
                        }

                        let timeline =
                            build_timeline(&pool, &name, created_at, is_running).await;

                        let started_ago = sample_started_ago(&docker, &id).await;

                        samples.push(DockerSample {
                            container_name: name,
                            image_repo,
                            version,
                            is_running,
                            health,
                            created_ago: humanize_ago(created_at),
                            started_ago,
                            timeline,
                        });
                    }
                }

                let _ = tx.send(MetricEvent::Docker(samples));
            }
            Err(err) => tracing::warn!(%err, "failed to list containers"),
        }

        tokio::time::sleep(interval).await;
    }
}

async fn sample_stats(docker: &Docker, id: &str) -> Option<(f64, f64)> {
    let options = StatsOptions { stream: false, one_shot: true };
    let mut stream = docker.stats(id, Some(options));
    let stats = stream.next().await?.ok()?;

    let cpu_usage = calc_cpu_percent(&stats);
    let ram_usage = stats.memory_stats.as_ref().and_then(|m| m.usage).unwrap_or(0) as f64;

    Some((cpu_usage, ram_usage))
}

/// Fetches the container's last start time via `inspect` and renders it the
/// same way as `created_ago`. Distinct from `created_ago` once a container
/// has been restarted. Returns `—` if the daemon has no start time on record
/// (never started) or the inspect call fails.
async fn sample_started_ago(docker: &Docker, id: &str) -> String {
    let Ok(inspect) = docker.inspect_container(id, None).await else {
        return "—".to_string();
    };
    let started_at = inspect.state.and_then(|s| s.started_at).unwrap_or_default();
    match parse_rfc3339_to_unix(&started_at) {
        Some(ts) if ts > 0 => humanize_ago(ts),
        _ => "—".to_string(),
    }
}

/// Parses a Docker-supplied RFC 3339 UTC timestamp (`2024-05-01T12:34:56.123456789Z`)
/// into Unix seconds, without pulling in a datetime crate for one field.
fn parse_rfc3339_to_unix(s: &str) -> Option<i64> {
    let s = s.trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;

    let mut d = date.splitn(3, '-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;

    let time = time.split('.').next().unwrap_or(time);
    let mut t = time.splitn(3, ':');
    let hour: i64 = t.next()?.parse().ok()?;
    let min: i64 = t.next()?.parse().ok()?;
    let sec: i64 = t.next()?.parse().ok()?;

    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + min * 60 + sec)
}

/// Howard Hinnant's `days_from_civil`: converts a Gregorian y/m/d into days
/// since the Unix epoch (1970-01-01), valid for the proleptic Gregorian calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn calc_cpu_percent(stats: &bollard::models::ContainerStatsResponse) -> f64 {
    let (Some(cpu), Some(precpu)) = (&stats.cpu_stats, &stats.precpu_stats) else {
        return 0.0;
    };
    let cpu_delta = cpu.cpu_usage.as_ref().and_then(|u| u.total_usage).unwrap_or(0) as f64
        - precpu.cpu_usage.as_ref().and_then(|u| u.total_usage).unwrap_or(0) as f64;
    let system_delta =
        cpu.system_cpu_usage.unwrap_or(0) as f64 - precpu.system_cpu_usage.unwrap_or(0) as f64;
    let online_cpus = cpu.online_cpus.unwrap_or(1) as f64;

    if system_delta > 0.0 && cpu_delta > 0.0 {
        (cpu_delta / system_delta) * online_cpus * 100.0
    } else {
        0.0
    }
}

/// Splits a Docker image reference into (repo, tag). Digests (`repo@sha256:...`)
/// are treated as untagged since the digest isn't a human version string.
fn split_image(image: &str) -> (String, String) {
    if let Some((repo, _digest)) = image.split_once('@') {
        return (repo_basename(repo), String::new());
    }
    match image.rsplit_once(':') {
        Some((repo, tag)) => (repo_basename(repo), tag.to_string()),
        None => (repo_basename(image), String::new()),
    }
}

fn repo_basename(repo: &str) -> String {
    repo.rsplit('/').next().unwrap_or(repo).to_string()
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn humanize_ago(created_at: i64) -> String {
    let diff = (now_unix() - created_at).max(0);
    if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// Builds the 24h uptime timeline: one CSS class per bucket, oldest first.
/// Buckets before `created_at` are `bar-na`; buckets with at least one
/// recorded sample are `bar-up`, otherwise `bar-down`. The last bucket is
/// always forced to reflect the container's current live status.
async fn build_timeline(
    pool: &Pool,
    container_name: &str,
    created_at: i64,
    is_running: bool,
) -> Vec<&'static str> {
    let timestamps =
        db::container_sample_timestamps(pool, container_name, TIMELINE_WINDOW_HOURS)
            .await
            .unwrap_or_default();

    let now = now_unix();
    let window_start = now - TIMELINE_WINDOW_SECS;
    let bucket_width = TIMELINE_WINDOW_SECS / TIMELINE_BUCKETS as i64;

    let mut buckets = vec!["bar-na"; TIMELINE_BUCKETS];
    for (i, bucket) in buckets.iter_mut().enumerate() {
        let bucket_start = window_start + i as i64 * bucket_width;
        let bucket_end = bucket_start + bucket_width;
        if bucket_end <= created_at {
            continue;
        }
        let has_sample = timestamps.iter().any(|&t| t >= bucket_start && t < bucket_end);
        *bucket = if has_sample { "bar-up" } else { "bar-down" };
    }

    if let Some(last) = buckets.last_mut() {
        *last = if is_running { "bar-up bar-live" } else { "bar-down bar-live" };
    }

    buckets
}
