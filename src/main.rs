mod calendar;
mod metrics;
mod untis;

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::Response,
    routing::get,
};
use calendar::Calendar;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{RwLock, watch};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Shared = Arc<RwLock<AppState>>;

#[derive(Default)]
struct AppState {
    calendar: Calendar,
    metrics: metrics::Metrics,
}

impl AppState {
    fn finish_refresh(
        &mut self,
        events: Result<BTreeMap<i64, String>>,
        now: jiff::Timestamp,
        started: Instant,
    ) -> Result<usize> {
        let result = events.and_then(|events| {
            let count = events.len();
            self.calendar.update(events, now)?;
            Ok(count)
        });
        self.metrics
            .record(result.as_ref().ok().copied(), now, started.elapsed());
        result
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    user: String,
    secret: String,
    feed_token: String,
}

impl Config {
    fn read() -> Result<Self> {
        let path = std::env::var("CONFIG_FILE")
            .unwrap_or_else(|_| "/etc/hhek-untis2ics/config.json".into());
        let config: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.user.trim().is_empty() || self.secret.trim().is_empty() {
            return Err("user and mobile key must not be empty".into());
        }
        if self.feed_token.len() != 64 || !self.feed_token.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("feed_token must be 64 random hexadecimal characters".into());
        }
        Ok(())
    }
}

async fn feed(State(calendar): State<Shared>) -> Response {
    let state = calendar.read().await;
    let calendar = &state.calendar;
    let (status, content_type, body) = if calendar.ready() {
        (
            StatusCode::OK,
            "text/calendar; charset=utf-8",
            calendar.body.clone(),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "text/plain; charset=utf-8",
            "Calendar unavailable; retry later.\n".into(),
        )
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header("referrer-policy", "no-referrer")
        .body(Body::from(body))
        .expect("static response headers are valid")
}

async fn health(State(calendar): State<Shared>) -> StatusCode {
    if calendar.read().await.calendar.ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn metrics(State(state): State<Shared>) -> Response {
    let state = state.read().await;
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .body(Body::from(state.metrics.render(state.calendar.ready())))
        .expect("static response headers are valid")
}

fn metrics_router(state: Shared) -> Router {
    Router::new()
        .route("/metrics", get(metrics))
        .with_state(state)
}

fn router(token: &str, calendar: Shared) -> Router {
    Router::new()
        .route(&format!("/{token}/calendar.ics"), get(feed))
        .route("/healthz", get(health))
        .with_state(calendar)
}

async fn shutdown() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler must be available");
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
}

async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    mut stop: watch::Receiver<()>,
) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = stop.changed().await;
        })
        .await
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::read()?;
    match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => {}
        [command] if command == "url" => {
            println!("http://127.0.0.1:8080/{}/calendar.ics", config.feed_token);
            return Ok(());
        }
        _ => return Err("usage: hhek-untis2ics [url]".into()),
    }
    let client = untis::Untis::new(config.user, &config.secret)?;
    let calendar = Shared::default();
    let app = router(&config.feed_token, calendar.clone());
    let monitoring = metrics_router(calendar.clone());
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    let metrics_listener = tokio::net::TcpListener::bind("0.0.0.0:9090").await?;
    let updater = tokio::spawn(async move {
        loop {
            let started = Instant::now();
            let events = async { calendar::render(&client.fetch().await?) }.await;
            let result =
                calendar
                    .write()
                    .await
                    .finish_refresh(events, jiff::Timestamp::now(), started);
            match result {
                Ok(count) => eprintln!("Refreshed {count} calendar events"),
                Err(error) => eprintln!("Calendar refresh failed: {error}"),
            }
            tokio::time::sleep(Duration::from_secs(900)).await;
        }
    });
    eprintln!("Calendar on :8080; metrics on :9090; refresh every 15 minutes");
    let (stop, wait) = watch::channel(());
    let signal = tokio::spawn(async move {
        shutdown().await;
        // Closing the channel drains both HTTP listeners.
        drop(stop);
    });
    let result = tokio::try_join!(
        serve(listener, app, wait.clone()),
        serve(metrics_listener, monitoring, wait),
    );
    updater.abort();
    signal.abort();
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_rejects_unsafe_feed_paths() {
        let mut c = Config {
            user: "student".into(),
            secret: "test".into(),
            feed_token: "ab".repeat(32),
        };
        assert!(c.validate().is_ok());
        c.feed_token = "short".into();
        assert!(c.validate().is_err());
        c.feed_token = "/".repeat(64);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejected_snapshot_is_counted_as_failure_without_losing_calendar() {
        let mut state = AppState::default();
        let now = jiff::Timestamp::now();
        state
            .finish_refresh(Ok(BTreeMap::new()), now, Instant::now())
            .unwrap();
        let body = state.calendar.body.clone();
        // Calendar serialization rejects revisions outside the ICS integer range.
        let too_far = "2100-01-01T00:00:00Z".parse().unwrap();
        let events = BTreeMap::from([(42, "SUMMARY:private timetable text\r\n".into())]);
        assert!(
            state
                .finish_refresh(Ok(events), too_far, Instant::now())
                .is_err()
        );
        assert_eq!(state.calendar.body, body);
        let metrics = state.metrics.render(state.calendar.ready());
        assert!(metrics.contains("hhek_untis2ics_refresh_failures_total 1\n"));
        assert!(metrics.contains("hhek_untis2ics_refresh_successes_total 1\n"));
        assert!(!metrics.contains("private timetable text"));
    }

    #[tokio::test]
    async fn http_authentication_readiness_and_calendar_headers() {
        let state = Shared::default();
        let token = "ab".repeat(32);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = router(&token, state.clone());
        let metrics_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let monitor_base = format!("http://{}", metrics_listener.local_addr().unwrap());
        let monitoring = metrics_router(state.clone());
        let (stop, wait) = watch::channel(());
        let server = tokio::spawn(async move {
            tokio::try_join!(
                serve(listener, app, wait.clone()),
                serve(metrics_listener, monitoring, wait)
            )
            .unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{base}/{token}/calendar.ics");
        let metrics_url = format!("{monitor_base}/metrics");
        let initial = client.get(&metrics_url).send().await.unwrap();
        assert_eq!(initial.status(), 200);
        assert_eq!(
            initial.headers()[header::CONTENT_TYPE],
            "text/plain; version=0.0.4; charset=utf-8"
        );
        assert!(
            initial
                .text()
                .await
                .unwrap()
                .contains("hhek_untis2ics_ready 0\n")
        );
        assert_eq!(
            client
                .get(format!("{base}/metrics"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .get(format!("{monitor_base}/{token}/calendar.ics"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .get(format!("{base}/calendar.ics"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .get(format!("{base}/{}/calendar.ics", "cd".repeat(32)))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(client.get(&url).send().await.unwrap().status(), 503);
        state
            .write()
            .await
            .finish_refresh(
                Ok(Default::default()),
                jiff::Timestamp::now(),
                Instant::now(),
            )
            .unwrap();
        let response = client.get(&url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/calendar; charset=utf-8"
        );
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        assert!(
            response
                .text()
                .await
                .unwrap()
                .ends_with("END:VCALENDAR\r\n")
        );
        assert_eq!(
            client
                .get(format!("{base}/healthz"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            client.head(&url).send().await.unwrap().content_length(),
            Some(0)
        );
        assert_eq!(client.post(&url).send().await.unwrap().status(), 405);
        {
            let mut snapshot = state.write().await;
            assert!(
                snapshot
                    .finish_refresh(
                        Err("upstream failure".into()),
                        jiff::Timestamp::now(),
                        Instant::now()
                    )
                    .is_err()
            );
            assert!(snapshot.calendar.ready());
            snapshot.calendar.refreshed = Some(Instant::now() - Duration::from_secs(3601));
        }
        assert_eq!(client.get(&url).send().await.unwrap().status(), 503);
        let response = client.get(&metrics_url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let metrics = response.text().await.unwrap();
        assert!(metrics.contains("hhek_untis2ics_ready 0\n"));
        assert!(metrics.contains("hhek_untis2ics_last_refresh_success 0\n"));
        assert!(metrics.contains("hhek_untis2ics_refresh_failures_total 1\n"));
        assert!(!metrics.contains(&token));
        assert!(!metrics.contains("BEGIN:VCALENDAR"));
        drop(client);
        drop(stop);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}
