use crate::state::AppState;
use crate::suggestions::{ApplyRequest, ApplyResponse, Proposal};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use powerwatch_core::json_snapshot::{build_json_snapshot, JsonSnapshot};
use powerwatch_core::model::{Component, SensorReading};
use powerwatch_core::storage::AggregatedReading;
use serde::{Deserialize, Serialize};

fn convert_rss_to_mb(output: &str) -> String {
    let mut result = String::new();
    for (i, line) in output.lines().enumerate() {
        if i == 0 {
            result.push_str("    PID %CPU %MEM MEM(MB) COMMAND\n");
        } else {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 {
                let pid = parts[0];
                let cpu = parts[1];
                let mem = parts[2];
                let rss_kb: u64 = parts[3].parse().unwrap_or(0);
                let rss_mb = rss_kb / 1024;
                let comm = parts[4];
                result.push_str(&format!(
                    "{:>6} {:>5} {:>5} {:>7} {}\n",
                    pid, cpu, mem, rss_mb, comm
                ));
            } else {
                result.push_str(line);
            }
        }
    }
    result
}

const INDEX_HTML: &str = include_str!("../static/index.html");

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/alerts", get(crate::alerts::page))
        .route("/api/alerts", get(crate::alerts::overview).put(crate::alerts::update))
        .route("/api/alerts/test", post(crate::alerts::test_notification))
        .route("/api/health", get(health))
        .route("/api/snapshot", get(snapshot))
        .route("/api/history", get(history))
        .route("/api/history/range", get(history_range))
        .route("/api/suggestions", get(suggestions_list))
        .route("/api/suggestions/apply", post(suggestions_apply))
        .route("/api/processes/top", get(top_processes))
        .with_state(state)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(INDEX_HTML),
    )
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn snapshot(State(state): State<AppState>) -> Json<JsonSnapshot> {
    let snapshot = state.latest_snapshot.read().unwrap();
    Json(build_json_snapshot(&snapshot))
}

#[derive(Deserialize)]
struct HistoryParams {
    since: Option<String>,
}

async fn history(
    State(state): State<AppState>,
    Query(params): Query<HistoryParams>,
) -> Result<Json<Vec<SensorReading>>, (StatusCode, String)> {
    let Some(since_text) = params.since else {
        return Err((
            StatusCode::BAD_REQUEST,
            "missing ?since=... e.g. ?since=1h".to_string(),
        ));
    };

    let lookback = powerwatch_core::duration::parse_duration(&since_text)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(lookback).unwrap_or(chrono::Duration::zero());

    let storage_guard = state.storage.lock().unwrap();
    let readings = match storage_guard.as_ref() {
        Some(storage) => storage
            .readings_since(cutoff)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?,
        None => Vec::new(),
    };

    Ok(Json(readings))
}


#[derive(Deserialize)]
struct RangeHistoryParams {
    amount: Option<u32>,
    unit: Option<String>,
    max_points: Option<u32>,
}

#[derive(Serialize)]
struct HistorySummary {
    avg_watts: f64,
    min_watts: f64,
    max_watts: f64,
    energy_kwh: f64,
    samples: u64,
}

#[derive(Serialize)]
struct RangeHistoryResponse {
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    bucket_seconds: u64,
    summary: Option<HistorySummary>,
    points: Vec<AggregatedReading>,
}

fn range_start(
    now: chrono::DateTime<chrono::Utc>,
    amount: u32,
    unit: &str,
) -> Result<chrono::DateTime<chrono::Utc>, String> {
    if amount == 0 || amount > 10_000 {
        return Err("amount must be between 1 and 10000".to_string());
    }

    match unit.trim().to_ascii_lowercase().as_str() {
        "minute" | "minutes" | "min" => Ok(now - chrono::Duration::minutes(amount as i64)),
        "hour" | "hours" | "h" => Ok(now - chrono::Duration::hours(amount as i64)),
        "day" | "days" | "d" => Ok(now - chrono::Duration::days(amount as i64)),
        "week" | "weeks" | "w" => Ok(now - chrono::Duration::weeks(amount as i64)),
        "month" | "months" => now
            .checked_sub_months(chrono::Months::new(amount))
            .ok_or_else(|| "requested month range is out of bounds".to_string()),
        "year" | "years" | "y" => {
            let months = amount
                .checked_mul(12)
                .ok_or_else(|| "requested year range is too large".to_string())?;
            now.checked_sub_months(chrono::Months::new(months))
                .ok_or_else(|| "requested year range is out of bounds".to_string())
        }
        other => Err(format!(
            "unsupported unit '{other}' (use minutes, hours, days, weeks, months or years)"
        )),
    }
}

fn choose_bucket_seconds(range_seconds: u64, max_points: u32) -> u64 {
    let max_points = max_points.clamp(100, 5_000) as u64;
    let target = range_seconds.div_ceil(max_points).max(60);
    const BUCKETS: &[u64] = &[
        60,
        120,
        300,
        600,
        900,
        1_800,
        3_600,
        7_200,
        10_800,
        21_600,
        43_200,
        86_400,
        172_800,
        604_800,
        2_592_000,
    ];

    BUCKETS
        .iter()
        .copied()
        .find(|bucket| *bucket >= target)
        .unwrap_or(target)
}

fn summarize_history(
    points: &[AggregatedReading],
    bucket_seconds: u64,
) -> Option<HistorySummary> {
    let totals: Vec<_> = points
        .iter()
        .filter(|point| point.component == Component::Total)
        .collect();

    if totals.is_empty() {
        return None;
    }

    let samples: u64 = totals.iter().map(|point| point.samples).sum();
    let weighted_sum: f64 = totals
        .iter()
        .map(|point| point.avg_watts * point.samples as f64)
        .sum();
    let avg_watts = if samples > 0 {
        weighted_sum / samples as f64
    } else {
        0.0
    };
    let min_watts = totals
        .iter()
        .map(|point| point.min_watts)
        .fold(f64::INFINITY, f64::min);
    let max_watts = totals
        .iter()
        .map(|point| point.max_watts)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut sorted_totals = totals;
    sorted_totals.sort_by_key(|point| point.timestamp);
    let mut energy_kwh = 0.0;
    for pair in sorted_totals.windows(2) {
        let previous = pair[0];
        let next = pair[1];
        let delta_seconds = (next.timestamp - previous.timestamp).num_seconds();
        if delta_seconds <= 0 { continue; }
        let covered_seconds = (delta_seconds as u64).min(bucket_seconds);
        let average_watts = (previous.avg_watts + next.avg_watts) / 2.0;
        energy_kwh += average_watts * covered_seconds as f64 / 3_600_000.0;
    }

    Some(HistorySummary {
        avg_watts,
        min_watts,
        max_watts,
        energy_kwh,
        samples,
    })
}

async fn history_range(
    State(state): State<AppState>,
    Query(params): Query<RangeHistoryParams>,
) -> Result<Json<RangeHistoryResponse>, (StatusCode, String)> {
    let amount = params.amount.unwrap_or(24);
    let unit = params.unit.as_deref().unwrap_or("hours");
    let max_points = params.max_points.unwrap_or(1_200).clamp(100, 5_000);

    let to = chrono::Utc::now();
    let from = range_start(to, amount, unit).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let range_seconds = (to - from).num_seconds().max(1) as u64;
    let bucket_seconds = choose_bucket_seconds(range_seconds, max_points);

    let storage_guard = state.storage.lock().unwrap();
    let points = match storage_guard.as_ref() {
        Some(storage) => storage
            .aggregated_since(from, bucket_seconds)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")))?,
        None => Vec::new(),
    };

    let summary = summarize_history(&points, bucket_seconds);

    Ok(Json(RangeHistoryResponse {
        from,
        to,
        bucket_seconds,
        summary,
        points,
    }))
}

async fn suggestions_list(State(state): State<AppState>) -> Json<Vec<Proposal>> {
    let suggestions = state.suggestions;
    Json(suggestions.list_with_tokens())
}

async fn suggestions_apply(
    State(state): State<AppState>,
    Json(req): Json<ApplyRequest>,
) -> Result<Json<ApplyResponse>, (StatusCode, String)> {
    let suggestions = state.suggestions;
    let token = req.token;
    if token.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing token".to_string()));
    }

    match suggestions.apply(&token) {
        Ok(true) => Ok(Json(ApplyResponse {
            success: true,
            message: "action applied".to_string(),
        })),
        Ok(false) => Err((
            StatusCode::CONFLICT,
            "suggestion was already applied".to_string(),
        )),
        Err(e) => Err((StatusCode::BAD_REQUEST, e)),
    }
}

async fn top_processes() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("sh")
            .args(["-c", "ps -eo pid,pcpu,pmem,rss,comm --sort=-%cpu | head -6"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                let converted = convert_rss_to_mb(&text);
                Ok(Json(serde_json::json!({ "output": converted })))
            }
            Ok(out) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("command failed: {}", out.status),
            )),
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to run command: {}", e),
            )),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sh")
            .args(&["-c", "ps -eo pid,pcpu,pmem,rss,comm -m | head -6"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                let converted = convert_rss_to_mb(&text);
                Ok(Json(serde_json::json!({ "output": converted })))
            }
            Ok(out) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("command failed: {}", out.status),
            )),
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to run command: {}", e),
            )),
        }
    }

    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("powershell")
            .args(&["-command", "Get-Process | Sort-Object CPU -Descending | Select-Object -First 5 Id,CPU,WorkingSet,ProcessName | Format-Table -AutoSize"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                Ok(Json(serde_json::json!({ "output": text })))
            }
            Ok(out) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("command failed: {}", out.status),
            )),
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to run command: {}", e),
            )),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "not supported on this platform".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use powerwatch_core::model::{Component, Confidence};
    use powerwatch_core::sampler::Snapshot;
    use powerwatch_core::storage::Storage;
    use std::sync::{Arc, Mutex, RwLock};
    use tower::ServiceExt;

    fn test_state_with(snapshot: Snapshot) -> AppState {
        AppState {
            latest_snapshot: Arc::new(RwLock::new(snapshot)),
            storage: Arc::new(Mutex::new(None)),
            suggestions: crate::suggestions::SuggestionsState::new(),
            alerts: crate::alerts::AlertService::memory(),
        }
    }

    fn test_state_with_storage(snapshot: Snapshot, storage: Storage) -> AppState {
        AppState {
            latest_snapshot: Arc::new(RwLock::new(snapshot)),
            storage: Arc::new(Mutex::new(Some(storage))),
            suggestions: crate::suggestions::SuggestionsState::new(),
            alerts: crate::alerts::AlertService::memory(),
        }
    }

    fn empty_snapshot() -> Snapshot {
        Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![],
        }
    }

    fn reading(watts: f64) -> SensorReading {
        SensorReading {
            component: Component::Cpu,
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        }
    }

    async fn get_body_json(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn root_route_serves_the_frontend_html() {
        let app = build_router(test_state_with(empty_snapshot()));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap();
        assert!(content_type.to_str().unwrap().contains("text/html"));

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("PowerWatch"));
        assert!(text.contains("/api/snapshot"));
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let app = build_router(test_state_with(empty_snapshot()));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_unknown_route_returns_404() {
        let app = build_router(test_state_with(empty_snapshot()));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn snapshot_route_returns_the_current_state_as_json() {
        let state = test_state_with(Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(12.5)))],
        });
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["sensors"][0]["name"], "cpu");
        assert_eq!(json["sensors"][0]["reading"]["watts"], 12.5);
    }

    #[tokio::test]
    async fn snapshot_route_reflects_the_latest_state_when_it_changes() {
        let state = test_state_with(Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(10.0)))],
        });
        *state.latest_snapshot.write().unwrap() = Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![("cpu".to_string(), Ok(reading(99.0)))],
        };
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["sensors"][0]["reading"]["watts"], 99.0);
    }

    #[tokio::test]
    async fn history_without_since_is_a_bad_request() {
        let app = build_router(test_state_with(empty_snapshot()));

        let (status, _) = get_body_json(app, "/api/history").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn history_with_an_invalid_since_is_a_bad_request() {
        let app = build_router(test_state_with(empty_snapshot()));

        let (status, _) = get_body_json(app, "/api/history?since=not-a-duration").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn history_with_no_storage_returns_an_empty_list_not_an_error() {
        let app = build_router(test_state_with(empty_snapshot()));

        let (status, json) = get_body_json(app, "/api/history?since=1h").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn history_returns_readings_recorded_within_the_period() {
        let storage = Storage::open_in_memory().unwrap();
        storage.insert_reading(&reading(12.5)).unwrap();
        let state = test_state_with_storage(empty_snapshot(), storage);
        let app = build_router(state);

        let (status, json) = get_body_json(app, "/api/history?since=1h").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["watts"], 12.5);
    }

    #[tokio::test]
    async fn history_excludes_readings_older_than_the_period() {
        let storage = Storage::open_in_memory().unwrap();
        let old_reading = SensorReading {
            component: Component::Cpu,
            watts: 5.0,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now() - chrono::Duration::hours(2),
        };
        storage.insert_reading(&old_reading).unwrap();
        let state = test_state_with_storage(empty_snapshot(), storage);
        let app = build_router(state);

        let (status, json) = get_body_json(app, "/api/history?since=1h").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn suggestions_list_returns_empty_when_no_suggestions() {
        let state = test_state_with(empty_snapshot());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/suggestions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn suggestions_apply_without_token_is_bad_request() {
        let state = test_state_with(empty_snapshot());
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/suggestions/apply")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 400);
    }
}
