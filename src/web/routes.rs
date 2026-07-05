use std::convert::Infallible;
use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode, Uri};
use axum::response::sse::{Event, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::stream::{self, Stream};
use rust_embed::RustEmbed;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

use crate::alerts;
use crate::db::{self, Pool};
use crate::metrics::MetricEvent;
use crate::web::templates::{AlertsTemplate, DockerMetricsTemplate, IndexTemplate, VpsMetricsTemplate};

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

#[derive(Clone)]
pub struct AppState {
    pub tx: Arc<broadcast::Sender<MetricEvent>>,
    pub pool: Pool,
}

pub fn router(tx: Arc<broadcast::Sender<MetricEvent>>, pool: Pool) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/stream", get(stream))
        .route("/api/alerts/{id}/acknowledge", post(acknowledge_alert))
        .route("/assets/{*path}", get(asset))
        .with_state(AppState { tx, pool })
}

async fn index() -> impl IntoResponse {
    match IndexTemplate.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn asset(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches("/assets/");
    match Assets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(axum::body::Body::from(file.data))
                .unwrap()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Alerts only broadcast on state change (not on a fixed interval like
    // vps/docker), so a client connecting after the last change would never
    // see them without this snapshot sent immediately on connect.
    let active_alerts = db::list_active_alerts(&state.pool).await.unwrap_or_default();
    let initial_html = AlertsTemplate::from(active_alerts).render().unwrap_or_default();
    let initial = stream::once(std::future::ready(Ok(Event::default().event("alerts").data(initial_html))));

    let rx = state.tx.subscribe();
    let updates = BroadcastStream::new(rx).filter_map(|msg| {
        let event = match msg.ok()? {
            MetricEvent::Vps(sample) => {
                let html = VpsMetricsTemplate::from(sample).render().ok()?;
                Event::default().event("vps-metrics").data(html)
            }
            MetricEvent::Docker(sample) => {
                let html = DockerMetricsTemplate::from(sample).render().ok()?;
                Event::default().event("docker-metrics").data(html)
            }
            MetricEvent::Alerts(alerts) => {
                let html = AlertsTemplate::from(alerts).render().ok()?;
                Event::default().event("alerts").data(html)
            }
        };
        Some(Ok(event))
    });

    Sse::new(initial.chain(updates))
}

async fn acknowledge_alert(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    if let Err(err) = db::acknowledge_alert(&state.pool, id).await {
        tracing::warn!(%err, id, "failed to acknowledge alert");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    alerts::broadcast_active_alerts(&state.pool, &state.tx).await;
    StatusCode::NO_CONTENT
}
