use powerwatch_core::sampler::Snapshot;
use std::collections::{HashMap, VecDeque};

pub struct RollingWindow {
    capacity: usize,
    values: VecDeque<f64>,
}

impl RollingWindow {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            values: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, value: f64) {
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    pub fn values(&self) -> Vec<f64> {
        self.values.iter().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn min(&self) -> Option<f64> {
        self.values
            .iter()
            .copied()
            .fold(None, |acc, v| Some(acc.map_or(v, |m: f64| m.min(v))))
    }

    pub fn max(&self) -> Option<f64> {
        self.values
            .iter()
            .copied()
            .fold(None, |acc, v| Some(acc.map_or(v, |m: f64| m.max(v))))
    }
}

pub struct History {
    capacity: usize,
    windows: HashMap<String, RollingWindow>,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            windows: HashMap::new(),
        }
    }

    pub fn record(&mut self, snapshot: &Snapshot) {
        for (name, result) in &snapshot.results {
            if let Ok(reading) = result {
                self.windows
                    .entry(name.clone())
                    .or_insert_with(|| RollingWindow::new(self.capacity))
                    .push(reading.watts);
            }
        }
    }

    pub fn window(&self, name: &str) -> Option<&RollingWindow> {
        self.windows.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use powerwatch_core::model::{Component, Confidence, SensorReading};

    #[test]
    fn a_new_window_is_empty() {
        let window = RollingWindow::new(5);

        assert!(window.is_empty());
        assert_eq!(window.values(), Vec::<f64>::new());
    }

    #[test]
    fn pushed_values_come_back_in_order() {
        let mut window = RollingWindow::new(5);
        window.push(1.0);
        window.push(2.0);
        window.push(3.0);

        assert_eq!(window.values(), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn pushing_past_capacity_drops_the_oldest_value() {
        let mut window = RollingWindow::new(3);
        window.push(1.0);
        window.push(2.0);
        window.push(3.0);
        window.push(4.0);

        assert_eq!(window.values(), vec![2.0, 3.0, 4.0]);
        assert_eq!(window.len(), 3);
    }

    #[test]
    fn min_and_max_reflect_whats_currently_in_the_window() {
        let mut window = RollingWindow::new(5);
        window.push(4.0);
        window.push(1.0);
        window.push(7.0);

        assert_eq!(window.min(), Some(1.0));
        assert_eq!(window.max(), Some(7.0));
    }

    #[test]
    fn min_and_max_are_none_for_an_empty_window() {
        let window = RollingWindow::new(5);

        assert_eq!(window.min(), None);
        assert_eq!(window.max(), None);
    }

    fn reading(watts: f64) -> Result<SensorReading, powerwatch_core::model::SensorError> {
        Ok(SensorReading {
            component: Component::Cpu,
            watts,
            confidence: Confidence::Measured,
            timestamp: Utc::now(),
        })
    }

    #[test]
    fn recording_a_snapshot_creates_a_window_per_sensor_name() {
        let mut history = History::new(10);
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![("cpu".to_string(), reading(10.0))],
        };

        history.record(&snapshot);

        assert_eq!(history.window("cpu").unwrap().values(), vec![10.0]);
    }

    #[test]
    fn unknown_sensor_names_have_no_window() {
        let history = History::new(10);

        assert!(history.window("cpu").is_none());
    }

    #[test]
    fn a_failed_sensor_does_not_get_a_value_pushed() {
        let mut history = History::new(10);
        let snapshot = Snapshot {
            timestamp: Utc::now(),
            results: vec![(
                "gpu".to_string(),
                Err(powerwatch_core::model::SensorError::Unavailable(
                    "no driver".to_string(),
                )),
            )],
        };

        history.record(&snapshot);

        assert!(history.window("gpu").is_none());
    }

    #[test]
    fn recording_multiple_snapshots_accumulates_into_the_same_window() {
        let mut history = History::new(10);
        history.record(&Snapshot {
            timestamp: Utc::now(),
            results: vec![("cpu".to_string(), reading(10.0))],
        });
        history.record(&Snapshot {
            timestamp: Utc::now(),
            results: vec![("cpu".to_string(), reading(20.0))],
        });

        assert_eq!(history.window("cpu").unwrap().values(), vec![10.0, 20.0]);
    }
}
