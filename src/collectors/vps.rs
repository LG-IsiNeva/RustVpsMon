use std::path::Path;
use std::sync::Arc;

use sysinfo::{Disks, System};
use tokio::sync::broadcast;

use crate::alerts;
use crate::config::Config;
use crate::db::{self, Pool};
use crate::metrics::{MetricEvent, VpsSample};
use crate::notify::Notifier;

/// Polls host CPU/RAM/disk every `config.sample_interval()`, persists each
/// sample, checks alert thresholds, and broadcasts the sample for the SSE
/// route to render.
pub async fn run(
    pool: Pool,
    tx: broadcast::Sender<MetricEvent>,
    notifier: Arc<Notifier>,
    config: Arc<Config>,
) {
    let interval = config.sample_interval();
    let mut sys = System::new_all();

    loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let disks = Disks::new_with_refreshed_list();

        let cpu_usage = sys.global_cpu_usage() as f64;
        let ram_used = sys.used_memory();
        let ram_total = sys.total_memory();
        let (disk_used, disk_total) = disk_usage_at(&disks, &config.disk_path);

        let sample = VpsSample { cpu_usage, ram_used, ram_total, disk_used, disk_total };
        let mem_pct =
            if ram_total > 0 { ram_used as f64 / ram_total as f64 * 100.0 } else { 0.0 };
        let disk_pct =
            if disk_total > 0 { disk_used as f64 / disk_total as f64 * 100.0 } else { 0.0 };

        alerts::evaluate(
            &pool,
            &tx,
            &notifier,
            "vps_cpu",
            format!("CPU usage at {:.0}%", cpu_usage),
            cpu_usage > config.cpu_threshold_pct,
        )
        .await;
        alerts::evaluate(
            &pool,
            &tx,
            &notifier,
            "vps_ram",
            format!("RAM usage at {:.0}%", mem_pct),
            mem_pct > config.ram_threshold_pct,
        )
        .await;
        alerts::evaluate(
            &pool,
            &tx,
            &notifier,
            "vps_disk",
            format!("Disk usage at {:.0}%", disk_pct),
            disk_pct > config.disk_threshold_pct,
        )
        .await;

        if let Err(err) = db::insert_vps_metrics(
            &pool,
            sample.cpu_usage,
            sample.ram_used as f64,
            sample.disk_used as f64,
        )
        .await
        {
            tracing::warn!(%err, "failed to persist vps metrics");
        }

        let _ = tx.send(MetricEvent::Vps(sample));
        tokio::time::sleep(interval).await;
    }
}

/// Finds the disk whose mount point best matches `disk_path` (exact match
/// preferred, else the longest mount point that prefixes it — handles
/// nested mounts) and returns its `(used, total)` bytes. Falls back to
/// summing all disks if none match, so metrics degrade gracefully rather
/// than reporting zero.
fn disk_usage_at(disks: &Disks, disk_path: &str) -> (u64, u64) {
    let target = Path::new(disk_path);
    let best = disks
        .iter()
        .filter(|d| target.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len());

    match best {
        Some(d) => {
            let total = d.total_space();
            let available = d.available_space();
            (total - available, total)
        }
        None => disks.iter().fold((0u64, 0u64), |(used, total), d| {
            let t = d.total_space();
            let a = d.available_space();
            (used + (t - a), total + t)
        }),
    }
}
