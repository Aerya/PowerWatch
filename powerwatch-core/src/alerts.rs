use crate::sampler::Snapshot;
use std::time::{Duration, Instant};

pub struct AlertRule {
    pub component_name: String,
    pub threshold_watts: f64,
    pub sustained_for: Duration,
}

pub fn resolve_alert_rule(
    component: Option<&str>,
    above: Option<f64>,
    for_text: Option<&str>,
    run: Option<&str>,
) -> Result<Option<(AlertRule, Option<String>)>, String> {
    let Some(threshold_watts) = above else {
        if run.is_some() {
            return Err("--alert-run requires --alert-above".to_string());
        }
        if for_text.is_some() {
            return Err("--alert-for requires --alert-above".to_string());
        }
        if component.is_some() {
            return Err("--alert-component requires --alert-above".to_string());
        }
        return Ok(None);
    };

    let sustained_for = match for_text {
        Some(text) => crate::duration::parse_duration(text)?,
        None => Duration::from_secs(0),
    };

    let rule = AlertRule {
        component_name: component.unwrap_or("total").to_string(),
        threshold_watts,
        sustained_for,
    };

    Ok(Some((rule, run.map(|s| s.to_string()))))
}

pub struct AlertEvaluator {
    rule: AlertRule,
    above_since: Option<Instant>,
    fired: bool,
}

impl AlertEvaluator {
    pub fn new(rule: AlertRule) -> Self {
        Self {
            rule,
            above_since: None,
            fired: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.fired
    }

    fn current_watts(&self, snapshot: &Snapshot) -> Option<f64> {
        if self.rule.component_name == "total" {
            return snapshot.total().map(|r| r.watts);
        }

        snapshot
            .results
            .iter()
            .find(|(name, _)| name == &self.rule.component_name)
            .and_then(|(_, result)| result.as_ref().ok())
            .map(|r| r.watts)
    }

    pub fn evaluate_at(&mut self, snapshot: &Snapshot, now: Instant) -> bool {
        let watts = self.current_watts(snapshot);

        match watts {
            Some(w) if w > self.rule.threshold_watts => {
                let since = *self.above_since.get_or_insert(now);
                let elapsed = now.duration_since(since);

                if !self.fired && elapsed >= self.rule.sustained_for {
                    self.fired = true;
                    return true;
                }
                false
            }
            _ => {
                self.above_since = None;
                self.fired = false;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Component, Confidence, SensorError, SensorReading};
    use chrono::Utc;

    fn snapshot_with(component: Component, watts: f64) -> Snapshot {
        Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "cpu".to_string(),
                Ok(SensorReading {
                    component,
                    watts,
                    confidence: Confidence::Measured,
                    timestamp: Utc::now(),
                }),
            )],
        }
    }

    fn rule(threshold: f64, sustained_for: Duration) -> AlertRule {
        AlertRule {
            component_name: "cpu".to_string(),
            threshold_watts: threshold,
            sustained_for,
        }
    }

    #[test]
    fn never_fires_if_always_below_the_threshold() {
        let mut evaluator = AlertEvaluator::new(rule(50.0, Duration::from_secs(5)));
        let t0 = Instant::now();

        for i in 0..10 {
            let fired = evaluator.evaluate_at(
                &snapshot_with(Component::Cpu, 10.0),
                t0 + Duration::from_secs(i),
            );
            assert!(!fired);
        }
        assert!(!evaluator.is_active());
    }

    #[test]
    fn does_not_fire_before_the_sustained_duration_elapses() {
        let mut evaluator = AlertEvaluator::new(rule(50.0, Duration::from_secs(5)));
        let t0 = Instant::now();

        let fired = evaluator.evaluate_at(&snapshot_with(Component::Cpu, 80.0), t0);
        assert!(!fired);
        let fired = evaluator.evaluate_at(
            &snapshot_with(Component::Cpu, 80.0),
            t0 + Duration::from_secs(3),
        );
        assert!(!fired);
    }

    #[test]
    fn fires_exactly_once_when_the_duration_completes() {
        let mut evaluator = AlertEvaluator::new(rule(50.0, Duration::from_secs(5)));
        let t0 = Instant::now();

        evaluator.evaluate_at(&snapshot_with(Component::Cpu, 80.0), t0);
        let fired = evaluator.evaluate_at(
            &snapshot_with(Component::Cpu, 80.0),
            t0 + Duration::from_secs(5),
        );

        assert!(fired);
        assert!(evaluator.is_active());
    }

    #[test]
    fn does_not_fire_again_while_still_sustained() {
        let mut evaluator = AlertEvaluator::new(rule(50.0, Duration::from_secs(5)));
        let t0 = Instant::now();

        evaluator.evaluate_at(&snapshot_with(Component::Cpu, 80.0), t0);
        evaluator.evaluate_at(
            &snapshot_with(Component::Cpu, 80.0),
            t0 + Duration::from_secs(5),
        );
        let fired_again = evaluator.evaluate_at(
            &snapshot_with(Component::Cpu, 80.0),
            t0 + Duration::from_secs(10),
        );

        assert!(!fired_again);
        assert!(evaluator.is_active()); // still ongoing, just not re-firing
    }

    #[test]
    fn can_fire_again_after_dropping_and_re_crossing_the_threshold() {
        let mut evaluator = AlertEvaluator::new(rule(50.0, Duration::from_secs(5)));
        let t0 = Instant::now();

        evaluator.evaluate_at(&snapshot_with(Component::Cpu, 80.0), t0);
        evaluator.evaluate_at(
            &snapshot_with(Component::Cpu, 80.0),
            t0 + Duration::from_secs(5),
        );
        assert!(evaluator.is_active());
        evaluator.evaluate_at(
            &snapshot_with(Component::Cpu, 10.0),
            t0 + Duration::from_secs(6),
        );
        assert!(!evaluator.is_active());

        let t1 = t0 + Duration::from_secs(20);
        evaluator.evaluate_at(&snapshot_with(Component::Cpu, 80.0), t1);
        let fired_again = evaluator.evaluate_at(
            &snapshot_with(Component::Cpu, 80.0),
            t1 + Duration::from_secs(5),
        );

        assert!(fired_again);
    }

    #[test]
    fn a_rule_for_an_unknown_component_name_never_fires() {
        let mut evaluator = AlertEvaluator::new(AlertRule {
            component_name: "gpu".to_string(),
            threshold_watts: 1.0,
            sustained_for: Duration::from_secs(1),
        });
        let t0 = Instant::now();

        let fired = evaluator.evaluate_at(&snapshot_with(Component::Cpu, 999.0), t0);
        let fired_later = evaluator.evaluate_at(
            &snapshot_with(Component::Cpu, 999.0),
            t0 + Duration::from_secs(5),
        );

        assert!(!fired);
        assert!(!fired_later);
    }

    #[test]
    fn a_rule_for_a_sensor_that_failed_this_cycle_never_fires() {
        let mut evaluator = AlertEvaluator::new(rule(1.0, Duration::from_secs(1)));
        let t0 = Instant::now();
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "cpu".to_string(),
                Err(SensorError::Unavailable("no RAPL".to_string())),
            )],
        };

        let fired = evaluator.evaluate_at(&snapshot, t0);
        let fired_later = evaluator.evaluate_at(&snapshot, t0 + Duration::from_secs(5));

        assert!(!fired);
        assert!(!fired_later);
    }

    #[test]
    fn a_total_rule_watches_the_snapshots_combined_total() {
        let mut evaluator = AlertEvaluator::new(AlertRule {
            component_name: "total".to_string(),
            threshold_watts: 5.0,
            sustained_for: Duration::from_secs(1),
        });
        let t0 = Instant::now();
        let snapshot = snapshot_with(Component::Cpu, 10.0);

        evaluator.evaluate_at(&snapshot, t0);
        let fired = evaluator.evaluate_at(&snapshot, t0 + Duration::from_secs(1));

        assert!(fired);
    }

    #[test]
    fn no_alert_flags_means_no_rule_configured() {
        let result = resolve_alert_rule(None, None, None, None);

        assert!(result.unwrap().is_none());
    }

    #[test]
    fn alert_run_without_alert_above_is_an_error() {
        let result = resolve_alert_rule(None, None, None, Some("echo hi"));

        assert!(result.is_err());
    }

    #[test]
    fn alert_for_without_alert_above_is_an_error() {
        let result = resolve_alert_rule(None, None, Some("10s"), None);

        assert!(result.is_err());
    }

    #[test]
    fn alert_component_without_alert_above_is_an_error() {
        let result = resolve_alert_rule(Some("cpu"), None, None, None);

        assert!(result.is_err());
    }

    #[test]
    fn a_valid_rule_defaults_component_to_total_and_duration_to_zero() {
        let (rule, hook) = resolve_alert_rule(None, Some(50.0), None, None)
            .unwrap()
            .unwrap();

        assert_eq!(rule.component_name, "total");
        assert_eq!(rule.threshold_watts, 50.0);
        assert_eq!(rule.sustained_for, Duration::from_secs(0));
        assert!(hook.is_none());
    }

    #[test]
    fn a_fully_specified_rule_carries_every_field_through() {
        let (rule, hook) =
            resolve_alert_rule(Some("cpu"), Some(30.0), Some("10s"), Some("notify-send hi"))
                .unwrap()
                .unwrap();

        assert_eq!(rule.component_name, "cpu");
        assert_eq!(rule.threshold_watts, 30.0);
        assert_eq!(rule.sustained_for, Duration::from_secs(10));
        assert_eq!(hook.unwrap(), "notify-send hi");
    }

    #[test]
    fn an_invalid_duration_string_is_reported_clearly() {
        let result = resolve_alert_rule(None, Some(50.0), Some("not-a-duration"), None);

        assert!(result.is_err());
    }
}
