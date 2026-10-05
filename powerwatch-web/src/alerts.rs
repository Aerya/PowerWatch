use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::Json;
use powerwatch_core::sampler::Snapshot;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant};

const ALERTS_HTML: &str = include_str!("../static/alerts.html");
const MAX_EVENTS: usize = 50;
const MAX_RULES: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct NotificationConfig {
    pub discord_webhook: String,
    pub apprise_endpoint: String,
    pub apprise_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AlertRuleConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub component: String,
    pub threshold_watts: f64,
    pub sustained_seconds: u64,
    pub notify_recovery: bool,
}

impl Default for AlertRuleConfig {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "Power alert".to_string(),
            enabled: true,
            component: "total".to_string(),
            threshold_watts: 50.0,
            sustained_seconds: 30,
            notify_recovery: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct AlertSettings {
    pub notifications: NotificationConfig,
    pub rules: Vec<AlertRuleConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AlertEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub kind: String,
    pub rule_id: String,
    pub rule_name: String,
    pub component: String,
    pub watts: f64,
    pub threshold_watts: f64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AlertStatus {
    pub id: String,
    pub active: bool,
    pub last_watts: Option<f64>,
    pub above_for_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AlertOverview {
    pub settings: AlertSettings,
    pub statuses: Vec<AlertStatus>,
    pub events: Vec<AlertEvent>,
}

#[derive(Default)]
struct RuleRuntime {
    above_since: Option<Instant>,
    active: bool,
    last_watts: Option<f64>,
}

struct AlertState {
    settings: AlertSettings,
    runtime: HashMap<String, RuleRuntime>,
    events: VecDeque<AlertEvent>,
}

#[derive(Clone)]
pub(crate) struct AlertService {
    inner: Arc<Mutex<AlertState>>,
    config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NotificationResult {
    pub channel: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TestNotificationResponse {
    pub success: bool,
    pub results: Vec<NotificationResult>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct TestNotificationRequest {
    pub channel: Option<String>,
    pub notifications: Option<NotificationConfig>,
}

fn new_id() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect()
}

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".local/share/powerwatch/alerts.json")
}

fn validate_notifications(config: &NotificationConfig) -> Result<(), String> {
    let discord = config.discord_webhook.trim();
    if !discord.is_empty()
        && !(discord.starts_with("https://discord.com/api/webhooks/")
            || discord.starts_with("https://discordapp.com/api/webhooks/")
            || discord.starts_with("https://canary.discord.com/api/webhooks/"))
    {
        return Err("Discord webhook must be an official HTTPS webhook URL".to_string());
    }

    let apprise = config.apprise_endpoint.trim();
    if !apprise.is_empty() && !(apprise.starts_with("http://") || apprise.starts_with("https://")) {
        return Err("Apprise endpoint must start with http:// or https://".to_string());
    }

    for url in &config.apprise_urls {
        let value = url.trim();
        if !value.is_empty() && !value.contains("://") {
            return Err(format!("invalid Apprise notification URL: {value}"));
        }
    }

    Ok(())
}

fn normalize_settings(mut settings: AlertSettings) -> Result<AlertSettings, String> {
    if settings.rules.len() > MAX_RULES {
        return Err(format!("at most {MAX_RULES} alert rules are supported"));
    }
    validate_notifications(&settings.notifications)?;

    settings.notifications.discord_webhook = settings.notifications.discord_webhook.trim().to_string();
    settings.notifications.apprise_endpoint = settings.notifications.apprise_endpoint.trim().to_string();
    settings.notifications.apprise_urls = settings.notifications.apprise_urls
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();

    let mut ids = HashSet::new();
    for rule in &mut settings.rules {
        rule.name = rule.name.trim().to_string();
        rule.component = rule.component.trim().to_ascii_lowercase();

        if rule.name.is_empty() {
            return Err("alert rule name cannot be empty".to_string());
        }
        if rule.component.is_empty() {
            return Err(format!("alert '{}' has no component", rule.name));
        }
        if !rule.threshold_watts.is_finite() || rule.threshold_watts < 0.0 {
            return Err(format!("alert '{}' threshold must be a non-negative number", rule.name));
        }
        if rule.threshold_watts > 100_000.0 {
            return Err(format!("alert '{}' threshold is unreasonably high", rule.name));
        }
        if rule.sustained_seconds > 7 * 24 * 60 * 60 {
            return Err(format!("alert '{}' sustained duration cannot exceed 7 days", rule.name));
        }

        if rule.id.trim().is_empty() || ids.contains(rule.id.trim()) {
            loop {
                let candidate = new_id();
                if !ids.contains(&candidate) {
                    rule.id = candidate;
                    break;
                }
            }
        } else {
            rule.id = rule.id.trim().to_string();
        }
        ids.insert(rule.id.clone());
    }

    Ok(settings)
}

fn persist_settings(path: &Path, settings: &AlertSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create alert config directory: {e}"))?;
    }
    let data = serde_json::to_vec_pretty(settings)
        .map_err(|e| format!("failed to serialize alert settings: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("failed to write alert settings: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("failed to save alert settings: {e}"))?;
    Ok(())
}

impl AlertService {
    pub(crate) fn load_default() -> Self {
        let path = default_config_path();
        match Self::load(path.clone()) {
            Ok(service) => service,
            Err(e) => {
                eprintln!("warning: failed to load alert settings from {}: {e}", path.display());
                Self::memory()
            }
        }
    }

    pub(crate) fn memory() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AlertState {
                settings: AlertSettings::default(),
                runtime: HashMap::new(),
                events: VecDeque::new(),
            })),
            config_path: None,
        }
    }

    fn load(path: PathBuf) -> Result<Self, String> {
        let settings = if path.exists() {
            let data = std::fs::read(&path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            serde_json::from_slice::<AlertSettings>(&data)
                .map_err(|e| format!("failed to parse {}: {e}", path.display()))?
        } else {
            AlertSettings::default()
        };
        let settings = normalize_settings(settings)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(AlertState {
                settings,
                runtime: HashMap::new(),
                events: VecDeque::new(),
            })),
            config_path: Some(path),
        })
    }

    pub(crate) fn overview(&self) -> AlertOverview {
        let state = self.inner.lock().unwrap();
        let statuses = state.settings.rules.iter().map(|rule| {
            let runtime = state.runtime.get(&rule.id);
            AlertStatus {
                id: rule.id.clone(),
                active: runtime.is_some_and(|runtime| runtime.active),
                last_watts: runtime.and_then(|runtime| runtime.last_watts),
                above_for_seconds: runtime
                    .and_then(|runtime| runtime.above_since)
                    .map(|since| since.elapsed().as_secs())
                    .unwrap_or(0),
            }
        }).collect();

        AlertOverview {
            settings: state.settings.clone(),
            statuses,
            events: state.events.iter().cloned().collect(),
        }
    }

    pub(crate) fn update(&self, settings: AlertSettings) -> Result<AlertOverview, String> {
        let settings = normalize_settings(settings)?;
        if let Some(path) = &self.config_path {
            persist_settings(path, &settings)?;
        }
        let mut state = self.inner.lock().unwrap();
        state.settings = settings;
        state.runtime.clear();
        drop(state);
        Ok(self.overview())
    }

    pub(crate) fn evaluate(&self, snapshot: &Snapshot) {
        let now = Instant::now();
        let mut emitted = Vec::new();
        let notifications;

        {
            let mut state = self.inner.lock().unwrap();
            notifications = state.settings.notifications.clone();
            let rules = state.settings.rules.clone();

            for rule in rules {
                let watts = watts_for(snapshot, &rule.component);
                let event = {
                    let runtime = state.runtime.entry(rule.id.clone()).or_default();
                    runtime.last_watts = watts;

                    if !rule.enabled {
                        runtime.above_since = None;
                        runtime.active = false;
                        None
                    } else if let Some(watts) = watts {
                        if watts > rule.threshold_watts {
                            let since = *runtime.above_since.get_or_insert(now);
                            if !runtime.active && now.duration_since(since).as_secs() >= rule.sustained_seconds {
                                runtime.active = true;
                                Some(AlertEvent {
                                    timestamp: chrono::Utc::now(),
                                    kind: "triggered".to_string(),
                                    rule_id: rule.id.clone(),
                                    rule_name: rule.name.clone(),
                                    component: rule.component.clone(),
                                    watts,
                                    threshold_watts: rule.threshold_watts,
                                    message: format!(
                                        "{}: {} is {:.1} W (threshold {:.1} W for {}s)",
                                        rule.name, rule.component, watts, rule.threshold_watts, rule.sustained_seconds
                                    ),
                                })
                            } else {
                                None
                            }
                        } else {
                            runtime.above_since = None;
                            if runtime.active {
                                runtime.active = false;
                                if rule.notify_recovery {
                                    Some(AlertEvent {
                                        timestamp: chrono::Utc::now(),
                                        kind: "recovered".to_string(),
                                        rule_id: rule.id.clone(),
                                        rule_name: rule.name.clone(),
                                        component: rule.component.clone(),
                                        watts,
                                        threshold_watts: rule.threshold_watts,
                                        message: format!(
                                            "{} recovered: {} is {:.1} W (threshold {:.1} W)",
                                            rule.name, rule.component, watts, rule.threshold_watts
                                        ),
                                    })
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    } else {
                        runtime.above_since = None;
                        runtime.active = false;
                        None
                    }
                };

                if let Some(event) = event {
                    emitted.push(event);
                }
            }

            for event in &emitted {
                state.events.push_front(event.clone());
            }
            while state.events.len() > MAX_EVENTS {
                state.events.pop_back();
            }
        }

        for event in emitted {
            let notifications = notifications.clone();
            thread::spawn(move || {
                for result in notify_for_event(&notifications, &event) {
                    if !result.ok {
                        eprintln!(
                            "alert notification failed on {}: {}",
                            result.channel,
                            result.error.unwrap_or_else(|| "unknown error".to_string())
                        );
                    }
                }
            });
        }
    }
}

fn watts_for(snapshot: &Snapshot, component: &str) -> Option<f64> {
    if component == "total" {
        return snapshot.total().map(|reading| reading.watts);
    }

    snapshot.results.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(component))
        .and_then(|(_, result)| result.as_ref().ok())
        .map(|reading| reading.watts)
}

fn send_json(url: &str, payload: &serde_json::Value) -> Result<(), String> {
    let body = serde_json::to_vec(payload)
        .map_err(|e| format!("failed to encode notification: {e}"))?;

    let mut child = Command::new("curl")
        .args([
            "-fsS", "--connect-timeout", "3", "--max-time", "8",
            "-H", "Content-Type: application/json", "-X", "POST",
            "--data-binary", "@-",
        ])
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start curl: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&body)
            .map_err(|e| format!("failed to write notification body: {e}"))?;
    }

    let output = child.wait_with_output()
        .map_err(|e| format!("failed to wait for curl: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            stderr
        })
    }
}

fn send_discord(config: &NotificationConfig, title: &str, body: &str) -> NotificationResult {
    let url = config.discord_webhook.trim();
    if url.is_empty() {
        return NotificationResult {
            channel: "discord".to_string(),
            ok: false,
            error: Some("Discord webhook is not configured".to_string()),
        };
    }

    let payload = serde_json::json!({
        "username": "PowerWatch",
        "content": format!("**{title}**\n{body}"),
        "allowed_mentions": { "parse": [] }
    });

    match send_json(url, &payload) {
        Ok(()) => NotificationResult { channel: "discord".to_string(), ok: true, error: None },
        Err(error) => NotificationResult { channel: "discord".to_string(), ok: false, error: Some(error) },
    }
}

fn send_apprise(config: &NotificationConfig, title: &str, body: &str, notification_type: &str) -> NotificationResult {
    let endpoint = config.apprise_endpoint.trim();
    if endpoint.is_empty() {
        return NotificationResult {
            channel: "apprise".to_string(),
            ok: false,
            error: Some("Apprise endpoint is not configured".to_string()),
        };
    }

    let mut payload = serde_json::json!({
        "title": title,
        "body": body,
        "type": notification_type,
        "format": "text"
    });
    if !config.apprise_urls.is_empty() {
        payload["urls"] = serde_json::json!(config.apprise_urls);
    }

    match send_json(endpoint, &payload) {
        Ok(()) => NotificationResult { channel: "apprise".to_string(), ok: true, error: None },
        Err(error) => NotificationResult { channel: "apprise".to_string(), ok: false, error: Some(error) },
    }
}

fn notify_for_event(config: &NotificationConfig, event: &AlertEvent) -> Vec<NotificationResult> {
    let (title, notification_type) = if event.kind == "recovered" {
        ("PowerWatch recovery", "success")
    } else {
        ("PowerWatch alert", "warning")
    };

    let mut results = Vec::new();
    if !config.discord_webhook.trim().is_empty() {
        results.push(send_discord(config, title, &event.message));
    }
    if !config.apprise_endpoint.trim().is_empty() {
        results.push(send_apprise(config, title, &event.message, notification_type));
    }
    results
}

fn test_channels(config: &NotificationConfig, channel: &str) -> Result<TestNotificationResponse, String> {
    validate_notifications(config)?;
    let title = "PowerWatch notification test";
    let body = "PowerWatch notifications are configured correctly.";
    let mut results = Vec::new();

    match channel {
        "all" => {
            if !config.discord_webhook.trim().is_empty() {
                results.push(send_discord(config, title, body));
            }
            if !config.apprise_endpoint.trim().is_empty() {
                results.push(send_apprise(config, title, body, "info"));
            }
            if results.is_empty() {
                return Err("no notification provider is configured".to_string());
            }
        }
        "discord" => results.push(send_discord(config, title, body)),
        "apprise" => results.push(send_apprise(config, title, body, "info")),
        _ => return Err("channel must be one of: all, discord, apprise".to_string()),
    }

    Ok(TestNotificationResponse {
        success: results.iter().all(|result| result.ok),
        results,
    })
}

pub(crate) async fn page() -> impl IntoResponse {
    Html(ALERTS_HTML)
}

pub(crate) async fn overview(State(state): State<AppState>) -> Json<AlertOverview> {
    Json(state.alerts.overview())
}

pub(crate) async fn update(
    State(state): State<AppState>,
    Json(settings): Json<AlertSettings>,
) -> Result<Json<AlertOverview>, (StatusCode, String)> {
    state.alerts.update(settings)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

pub(crate) async fn test_notification(
    State(state): State<AppState>,
    Json(request): Json<TestNotificationRequest>,
) -> Result<Json<TestNotificationResponse>, (StatusCode, String)> {
    let config = request.notifications
        .unwrap_or_else(|| state.alerts.overview().settings.notifications);
    let channel = request.channel.as_deref().unwrap_or("all").to_ascii_lowercase();

    test_channels(&config, &channel)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use powerwatch_core::model::{Component, Confidence, SensorReading};

    fn snapshot(watts: f64) -> Snapshot {
        Snapshot {
            timestamp: chrono::Utc::now(),
            results: vec![(
                "cpu".to_string(),
                Ok(SensorReading {
                    component: Component::Cpu,
                    watts,
                    confidence: Confidence::Measured,
                    timestamp: chrono::Utc::now(),
                }),
            )],
        }
    }

    #[test]
    fn normalizes_missing_rule_ids() {
        let mut settings = AlertSettings::default();
        let mut a = AlertRuleConfig::default();
        a.id.clear();
        let mut b = AlertRuleConfig::default();
        b.id.clear();
        settings.rules = vec![a, b];

        let normalized = normalize_settings(settings).unwrap();
        assert!(!normalized.rules[0].id.is_empty());
        assert!(!normalized.rules[1].id.is_empty());
        assert_ne!(normalized.rules[0].id, normalized.rules[1].id);
    }

    #[test]
    fn rule_triggers_and_recovers() {
        let service = AlertService::memory();
        let mut settings = AlertSettings::default();
        settings.rules.push(AlertRuleConfig {
            id: "cpu-test".to_string(),
            name: "CPU test".to_string(),
            enabled: true,
            component: "cpu".to_string(),
            threshold_watts: 10.0,
            sustained_seconds: 0,
            notify_recovery: true,
        });
        service.update(settings).unwrap();

        service.evaluate(&snapshot(20.0));
        let first = service.overview();
        assert!(first.statuses[0].active);
        assert_eq!(first.events[0].kind, "triggered");

        service.evaluate(&snapshot(5.0));
        let second = service.overview();
        assert!(!second.statuses[0].active);
        assert_eq!(second.events[0].kind, "recovered");
    }
}
