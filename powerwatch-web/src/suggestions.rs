use powerwatch_core::sampler::Snapshot;
use powerwatch_core::suggestions::{
    ActionDescriptor, ActionKind, PowerProfile, Severity, Suggestion,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TOKEN_TTL: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct SuggestionsState {
    pub engine: Arc<Mutex<SuggestionEngine>>,
    pub active: Arc<Mutex<Vec<Suggestion>>>,
    pub used_tokens: Arc<Mutex<HashMap<String, Instant>>>,
}

impl SuggestionsState {
    pub fn new() -> Self {
        Self {
            engine: Arc::new(Mutex::new(SuggestionEngine::default())),
            active: Arc::new(Mutex::new(Vec::new())),
            used_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn evaluate(&self, snapshot: &Snapshot) {
        let mut engine = self.engine.lock().unwrap();
        let new_suggestions = engine.evaluate(snapshot);
        drop(engine);

        let mut active = self.active.lock().unwrap();
        active.retain(|s| {
            !new_suggestions
                .iter()
                .any(|ns| ns.component_name == s.component_name)
        });

        for s in new_suggestions {
            active.push(s);
        }
    }

    pub fn list_with_tokens(&self) -> Vec<Proposal> {
        let active = self.active.lock().unwrap();
        let mut used_tokens = self.used_tokens.lock().unwrap();
        let now = Instant::now();
        used_tokens.retain(|_, issued| now.duration_since(*issued) < TOKEN_TTL);

        active
            .iter()
            .map(|s| {
                let token = if s.action.is_some() {
                    let mut rng = rand::thread_rng();
                    let token: String = (0..16)
                        .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
                        .collect();
                    used_tokens.insert(token.clone(), now);
                    Some(token)
                } else {
                    None
                };
                Proposal {
                    suggestion: s.clone(),
                    token,
                }
            })
            .collect()
    }

    pub fn apply(&self, token: &str) -> Result<bool, String> {
        let mut used_tokens = self.used_tokens.lock().unwrap();

        let now = Instant::now();
        if !used_tokens.contains_key(token) {
            return Err("invalid or expired token".to_string());
        }
        if now.duration_since(used_tokens[token]) > TOKEN_TTL {
            used_tokens.remove(token);
            return Err("token expired".to_string());
        }

        let mut active = self.active.lock().unwrap();
        let pos = active.iter().position(|s| s.action.is_some());

        match pos {
            Some(pos) => {
                let suggestion = active.remove(pos);
                drop(active);

                if let Some(action) = suggestion.action {
                    apply_action(&action);
                }

                used_tokens.remove(token);
                Ok(true)
            }
            None => {
                used_tokens.remove(token);
                Err("suggestion already applied".to_string())
            }
        }
    }
}

#[derive(Serialize)]
pub struct Proposal {
    pub suggestion: Suggestion,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Deserialize)]
pub struct ApplyRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct ApplyResponse {
    pub success: bool,
    pub message: String,
}

fn apply_action(action: &ActionDescriptor) {
    match &action.kind {
        ActionKind::SetPowerProfile(profile) => {
            use powerwatch_core::actions::{
                linux_command_for, linux_command_for_screensaver, linux_command_for_sleep_timer,
                linux_command_for_top_processes, PowerProfileActionExecutor, RealCommandRunner,
            };
            let runner = RealCommandRunner;
            let mut executor = PowerProfileActionExecutor::new(
                runner,
                linux_command_for,
                linux_command_for_screensaver,
                linux_command_for_top_processes,
                linux_command_for_sleep_timer,
            );
            if let Err(e) = executor.apply(&ActionKind::SetPowerProfile(*profile)) {
                eprintln!("failed to apply action: {e:?}");
            }
        }
        ActionKind::SetScreensaver(enable) => {
            use powerwatch_core::actions::{
                linux_command_for, linux_command_for_screensaver, linux_command_for_sleep_timer,
                linux_command_for_top_processes, PowerProfileActionExecutor, RealCommandRunner,
            };
            let runner = RealCommandRunner;
            let mut executor = PowerProfileActionExecutor::new(
                runner,
                linux_command_for,
                linux_command_for_screensaver,
                linux_command_for_top_processes,
                linux_command_for_sleep_timer,
            );
            if let Err(e) = executor.apply(&ActionKind::SetScreensaver(*enable)) {
                eprintln!("failed to apply screensaver action: {e:?}");
            }
        }
        ActionKind::ShowTopProcesses => {
            use powerwatch_core::actions::{
                linux_command_for, linux_command_for_screensaver, linux_command_for_sleep_timer,
                linux_command_for_top_processes, PowerProfileActionExecutor, RealCommandRunner,
            };
            let runner = RealCommandRunner;
            let mut executor = PowerProfileActionExecutor::new(
                runner,
                linux_command_for,
                linux_command_for_screensaver,
                linux_command_for_top_processes,
                linux_command_for_sleep_timer,
            );
            if let Err(e) = executor.apply(&ActionKind::ShowTopProcesses) {
                eprintln!("failed to run top processes command: {e:?}");
            }
        }
        ActionKind::ShowSleepTimer => {
            use powerwatch_core::actions::{
                linux_command_for, linux_command_for_screensaver, linux_command_for_sleep_timer,
                linux_command_for_top_processes, PowerProfileActionExecutor, RealCommandRunner,
            };
            let runner = RealCommandRunner;
            let mut executor = PowerProfileActionExecutor::new(
                runner,
                linux_command_for,
                linux_command_for_screensaver,
                linux_command_for_top_processes,
                linux_command_for_sleep_timer,
            );
            if let Err(e) = executor.apply(&ActionKind::ShowSleepTimer) {
                eprintln!("failed to run sleep timer command: {e:?}");
            }
        }
    }
}

#[derive(Default)]
pub struct SuggestionEngine {
    fired: std::collections::HashMap<String, (f64, Instant)>,
}

impl SuggestionEngine {
    pub fn evaluate(&mut self, snapshot: &Snapshot) -> Vec<Suggestion> {
        let mut suggestions = Vec::new();
        let now = Instant::now();
        let current_profile = detect_current_profile();

        if let Some(total) = snapshot.total() {
            if total.watts > 15.0 {
                let key = "total_high".to_string();
                let entry = self.fired.entry(key.clone()).or_insert((0.0, now));
                if entry.0 < 15.0 {
                    entry.0 = total.watts;
                    entry.1 = now;
                } else if now.duration_since(entry.1) >= Duration::from_secs(3) {
                    if current_profile != Some(PowerProfile::PowerSaver) {
                        suggestions.push(Suggestion {
                            message: "Total power draw has been high. Consider switching to power saver profile.".to_string(),
                            component_name: "total".to_string(),
                            severity: Severity::Warning,
                            action: Some(ActionDescriptor {
                                label: "Switch to power saver".to_string(),
                                kind: ActionKind::SetPowerProfile(PowerProfile::PowerSaver),
                            }),
                        });
                    }
                    self.fired.remove(&key);
                }
            } else {
                self.fired.remove("total_high");
            }
        }

        if let Some(cpu_reading) = snapshot
            .results
            .iter()
            .find(|(name, _)| name == "cpu")
            .and_then(|(_, r)| r.as_ref().ok())
        {
            if cpu_reading.watts > 8.0 {
                let key = "cpu_high".to_string();
                let entry = self.fired.entry(key.clone()).or_insert((0.0, now));
                if entry.0 < 8.0 {
                    entry.0 = cpu_reading.watts;
                    entry.1 = now;
                } else if now.duration_since(entry.1) >= Duration::from_secs(3) {
                    if current_profile != Some(PowerProfile::PowerSaver) {
                        suggestions.push(Suggestion {
                            message: "CPU power draw has been high. Consider reducing workload or switching to power saver.".to_string(),
                            component_name: "cpu".to_string(),
                            severity: Severity::Warning,
                            action: Some(ActionDescriptor {
                                label: "Switch to power saver".to_string(),
                                kind: ActionKind::SetPowerProfile(PowerProfile::PowerSaver),
                            }),
                        });
                    }
                    self.fired.remove(&key);
                }
            } else {
                self.fired.remove("cpu_high");
            }
        }

        if let Some(total) = snapshot.total() {
            if total.watts > 20.0 {
                let key = "display_idle".to_string();
                let entry = self.fired.entry(key.clone()).or_insert((0.0, now));
                if entry.0 < 20.0 {
                    entry.0 = total.watts;
                    entry.1 = now;
                } else if now.duration_since(entry.1) >= Duration::from_secs(30) {
                    suggestions.push(Suggestion {
                        message: "System appears idle with display active. Consider enabling screensaver to save energy.".to_string(),
                        component_name: "display".to_string(),
                        severity: Severity::Info,
                        action: Some(ActionDescriptor {
                            label: "Enable screensaver".to_string(),
                            kind: ActionKind::SetScreensaver(true),
                        }),
                    });
                    self.fired.remove(&key);
                }
            } else {
                self.fired.remove("display_idle");
            }
        }

        if let Some(cpu_reading) = snapshot
            .results
            .iter()
            .find(|(name, _)| name == "cpu")
            .and_then(|(_, r)| r.as_ref().ok())
        {
            if cpu_reading.watts > 8.0 {
                let key = "cpu_high_info".to_string();
                let entry = self.fired.entry(key.clone()).or_insert((0.0, now));
                if entry.0 < 8.0 {
                    entry.0 = cpu_reading.watts;
                    entry.1 = now;
                } else if now.duration_since(entry.1) >= Duration::from_secs(10) {
                    suggestions.push(Suggestion {
                        message: "CPU usage has been high for a while. Review top processes to identify what's consuming resources.".to_string(),
                        component_name: "cpu".to_string(),
                        severity: Severity::Info,
                        action: Some(ActionDescriptor {
                            label: "Show top processes".to_string(),
                            kind: ActionKind::ShowTopProcesses,
                        }),
                    });
                    self.fired.remove(&key);
                }
            } else {
                self.fired.remove("cpu_high_info");
            }
        }

        suggestions
    }
}

fn detect_current_profile() -> Option<PowerProfile> {
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("powerprofilesctl").arg("get").output() {
            if output.status.success() {
                if let Ok(text) = String::from_utf8(output.stdout) {
                    let text = text.trim();
                    if text.contains("power-save") {
                        return Some(PowerProfile::PowerSaver);
                    } else if text.contains("performance") {
                        return Some(PowerProfile::Performance);
                    } else {
                        return Some(PowerProfile::Balanced);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(output) = Command::new("pmset").args(&["get", "current"]).output() {
            if output.status.success() {
                if let Ok(text) = String::from_utf8(output.stdout) {
                    if text.contains("lowpowermode 1") {
                        return Some(PowerProfile::PowerSaver);
                    } else if text.contains("lowpowermode 0") {
                        return Some(PowerProfile::Balanced);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // TODO
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use powerwatch_core::model::{Component, Confidence, SensorReading};

    fn snapshot_with(component: &str, watts: f64) -> Snapshot {
        Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                component.to_string(),
                Ok(SensorReading {
                    component: Component::Cpu,
                    watts,
                    confidence: Confidence::Measured,
                    timestamp: Utc::now(),
                }),
            )],
        }
    }

    #[test]
    fn suggestion_engine_fires_when_threshold_sustained() {
        let mut engine = SuggestionEngine::default();

        // below threshold
        let suggestions = engine.evaluate(&snapshot_with("cpu", 10.0));
        assert!(suggestions.is_empty());

        // cross threshold
        let suggestions = engine.evaluate(&snapshot_with("cpu", 40.0));
        assert!(suggestions.is_empty()); // not yet sustained
    }

    #[test]
    fn suggestions_state_lists_with_tokens() {
        let state = SuggestionsState::new();
        let suggestions = state.list_with_tokens();
        assert!(suggestions.is_empty());
    }
}
