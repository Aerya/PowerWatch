use powerwatch_core::sampler::{Sampler, Snapshot};
use powerwatch_core::storage::Storage;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct AppState {
    pub latest_snapshot: Arc<RwLock<Snapshot>>,
    pub storage: Arc<Mutex<Option<Storage>>>,
    pub suggestions: super::suggestions::SuggestionsState,
    pub alerts: super::alerts::AlertService,
    pub auth: super::auth::AuthService,
}

pub fn start_sampling_loop(
    mut sampler: Sampler,
    sample_interval: Duration,
    storage: Option<Storage>,
    log_continuously: bool,
    history_interval: Duration,
    suggestions_enabled: bool,
) -> AppState {
    let initial = sampler.sample_all();
    let latest_snapshot = Arc::new(RwLock::new(initial));
    let storage = Arc::new(Mutex::new(storage));
    let suggestions = super::suggestions::SuggestionsState::new();
    let alerts = super::alerts::AlertService::load_default();
    let alerts_loop = alerts.clone();
    let state = AppState {
        latest_snapshot: latest_snapshot.clone(),
        storage: storage.clone(),
        suggestions: suggestions.clone(),
        alerts,
        auth: super::auth::AuthService::disabled(),
    };

    thread::spawn(move || {
        let history_interval = history_interval.max(Duration::from_secs(1));
        let mut last_history_write = Instant::now()
            .checked_sub(history_interval)
            .unwrap_or_else(Instant::now);
        let maintenance_interval = Duration::from_secs(60 * 60);
        let mut last_history_maintenance = Instant::now()
            .checked_sub(maintenance_interval)
            .unwrap_or_else(Instant::now);

        loop {
            thread::sleep(sample_interval);
            let snapshot = sampler.sample_all();

            if suggestions_enabled {
                suggestions.evaluate(&snapshot);
            }
            alerts_loop.evaluate(&snapshot);

            if log_continuously && last_history_write.elapsed() >= history_interval {
                let readings: Vec<_> = snapshot.readings().cloned().collect();
                let total = snapshot.total();

                if let Ok(mut guard) = storage.lock() {
                    if let Some(db) = guard.as_mut() {
                        for reading in &readings {
                            let _ = db.insert_reading(reading);
                        }
                        if let Some(total) = total {
                            let _ = db.insert_reading(&total);
                            // The energy cursor is durable and independent of raw-history retention.
                            let gap = history_interval.as_secs().saturating_mul(3).max(60)
                                .min(i64::MAX as u64) as i64;
                            if let Err(error) = db.record_energy(&total,gap) {
                                eprintln!("warning: energy accounting failed: {error:?}");
                            }
                        }
                        if last_history_maintenance.elapsed() >= maintenance_interval {
                            let _ = db.compact_history(chrono::Utc::now());
                            last_history_maintenance = Instant::now();
                        }
                    }
                }

                last_history_write = Instant::now();
            }

            if let Ok(mut guard) = latest_snapshot.write() {
                *guard = snapshot;
            }
        }
    });

    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use powerwatch_core::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    fn reading(watts: f64) -> SensorReading {
        SensorReading {
            component: Component::Cpu,
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        }
    }

    struct FixedSensor {
        reading: SensorReading,
    }

    impl PowerSensor for FixedSensor {
        fn sample(&mut self) -> Result<SensorReading, SensorError> {
            Ok(self.reading.clone())
        }
    }

    struct CountingSensor {
        calls: Arc<AtomicU32>,
    }

    impl PowerSensor for CountingSensor {
        fn sample(&mut self) -> Result<SensorReading, SensorError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(reading(n as f64))
        }
    }

    #[test]
    fn the_returned_state_has_a_valid_reading_immediately() {
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(FixedSensor {
                reading: reading(42.0),
            }),
        );

        let state = start_sampling_loop(
            sampler,
            Duration::from_secs(60),
            None,
            false,
            Duration::from_secs(60),
            true,
        );

        let snapshot = state.latest_snapshot.read().unwrap();
        assert_eq!(snapshot.results.len(), 1);
        assert_eq!(snapshot.results[0].1.as_ref().unwrap().watts, 42.0);
    }

    #[test]
    fn the_background_thread_keeps_sampling_over_time() {
        let calls = Arc::new(AtomicU32::new(0));
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(CountingSensor {
                calls: calls.clone(),
            }),
        );

        let state = start_sampling_loop(
            sampler,
            Duration::from_millis(20),
            None,
            false,
            Duration::from_secs(60),
            true,
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_second_sample = false;
        while Instant::now() < deadline {
            let watts = state.latest_snapshot.read().unwrap().results[0]
                .1
                .as_ref()
                .unwrap()
                .watts;
            if watts >= 1.0 {
                saw_second_sample = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(
            saw_second_sample,
            "expected the background loop to sample more than once"
        );
    }

    #[test]
    fn writes_to_storage_when_logging_is_enabled() {
        let storage = Storage::open_in_memory().unwrap();
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(FixedSensor {
                reading: reading(7.0),
            }),
        );

        let state = start_sampling_loop(
            sampler,
            Duration::from_millis(20),
            Some(storage),
            true,
            Duration::from_millis(20),
            true,
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut found_a_reading = false;
        while Instant::now() < deadline {
            let guard = state.storage.lock().unwrap();
            let db = guard.as_ref().unwrap();
            let readings = db
                .readings_since(chrono::Utc::now() - chrono::Duration::minutes(1))
                .unwrap();
            if !readings.is_empty() {
                found_a_reading = true;
                break;
            }
            drop(guard);
            thread::sleep(Duration::from_millis(10));
        }

        assert!(
            found_a_reading,
            "expected a reading to be written to storage"
        );
    }

    #[test]
    fn does_not_write_to_storage_when_logging_is_disabled() {
        let storage = Storage::open_in_memory().unwrap();
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(FixedSensor {
                reading: reading(7.0),
            }),
        );

        let state = start_sampling_loop(
            sampler,
            Duration::from_millis(20),
            Some(storage),
            false,
            Duration::from_millis(20),
            true,
        );
        thread::sleep(Duration::from_millis(200));

        let guard = state.storage.lock().unwrap();
        let db = guard.as_ref().unwrap();
        let readings = db
            .readings_since(chrono::Utc::now() - chrono::Duration::minutes(1))
            .unwrap();

        assert!(readings.is_empty());
    }

    #[test]
    fn each_loop_updates_the_live_snapshot_without_double_sampling() {
        let calls = Arc::new(AtomicU32::new(0));
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(CountingSensor {
                calls: calls.clone(),
            }),
        );

        let _state = start_sampling_loop(
            sampler,
            Duration::from_millis(50),
            None,
            false,
            Duration::from_secs(60),
            false,
        );

        thread::sleep(Duration::from_millis(180));
        let observed = calls.load(Ordering::SeqCst);

        assert!(
            (3..=6).contains(&observed),
            "expected roughly one sample per loop, got {observed}"
        );
    }
}
