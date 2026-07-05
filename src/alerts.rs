use std::sync::Arc;

use tokio::sync::broadcast;

use crate::db::{self, Pool};
use crate::metrics::MetricEvent;
use crate::notify::{self, Notifier};

/// Evaluates one metric/component against its breach state, applying the
/// dedup rule from the spec: a new `TRIGGERED` alert (and notification) is
/// only created if none is already open for this `component`; once the
/// metric drops back to normal, any open alert (`TRIGGERED` or
/// `ACKNOWLEDGED`) is resolved and a single "back to normal" notification is
/// sent. After every write the active-alerts panel is re-broadcast so the
/// UI stays in sync without waiting on a page reload.
pub async fn evaluate(
    pool: &Pool,
    tx: &broadcast::Sender<MetricEvent>,
    notifier: &Arc<Notifier>,
    component: &str,
    message: String,
    breached: bool,
) {
    let mut changed = false;

    if breached {
        match db::triggered_alert_for_component(pool, component).await {
            Ok(Some(_)) => {}
            Ok(None) => match db::insert_alert(pool, component, &message).await {
                Ok(_) => {
                    changed = true;
                    tokio::spawn(notify::notify_triggered(
                        notifier.clone(),
                        component.to_string(),
                        message.clone(),
                    ));
                }
                Err(err) => tracing::warn!(%err, component, "failed to insert alert"),
            },
            Err(err) => tracing::warn!(%err, component, "failed to query triggered alert"),
        }
    } else {
        match db::open_alert_for_component(pool, component).await {
            Ok(Some(alert)) => match db::resolve_alert(pool, alert.id).await {
                Ok(()) => {
                    changed = true;
                    tokio::spawn(notify::notify_resolved(
                        notifier.clone(),
                        component.to_string(),
                        message.clone(),
                    ));
                }
                Err(err) => tracing::warn!(%err, component, "failed to resolve alert"),
            },
            Ok(None) => {}
            Err(err) => tracing::warn!(%err, component, "failed to query open alert"),
        }
    }

    if changed {
        broadcast_active_alerts(pool, tx).await;
    }
}

/// Re-fetches the active alert list and pushes it over the broadcast
/// channel. Called both from `evaluate` (on state change) and from the
/// acknowledge route handler (so the ack is reflected immediately).
pub async fn broadcast_active_alerts(pool: &Pool, tx: &broadcast::Sender<MetricEvent>) {
    match db::list_active_alerts(pool).await {
        Ok(alerts) => {
            let _ = tx.send(MetricEvent::Alerts(alerts));
        }
        Err(err) => tracing::warn!(%err, "failed to list active alerts"),
    }
}
