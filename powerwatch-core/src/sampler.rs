use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use chrono::Utc;

pub struct Sampler {
    sensors: Vec<(String, Box<dyn PowerSensor>)>,
}

impl Sampler {
    pub fn new() -> Self {
        Self {
            sensors: Vec::new(),
        }
    }

    pub fn add_sensor(&mut self, name: impl Into<String>, sensor: Box<dyn PowerSensor>) {
        self.sensors.push((name.into(), sensor));
    }

    pub fn is_empty(&self) -> bool {
        self.sensors.is_empty()
    }

    pub fn sample_all(&mut self) -> Snapshot {
        let results = self
            .sensors
            .iter_mut()
            .map(|(name, sensor)| (name.clone(), sensor.sample()))
            .collect();

        Snapshot {
            timestamp: Utc::now(),
            results,
        }
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Snapshot {
    pub timestamp: chrono::DateTime<Utc>,
    pub results: Vec<(String, Result<SensorReading, SensorError>)>,
}

impl Snapshot {
    pub fn readings(&self) -> impl Iterator<Item = &SensorReading> {
        self.results.iter().filter_map(|(_, r)| r.as_ref().ok())
    }

    pub fn total_watts(&self) -> f64 {
        self.readings().map(|r| r.watts).sum()
    }

    pub fn total(&self) -> Option<SensorReading> {
        let mut count = 0;
        let mut any_estimated = false;

        for reading in self.readings() {
            count += 1;
            if reading.confidence == Confidence::Estimated {
                any_estimated = true;
            }
        }

        if count == 0 {
            return None;
        }

        Some(SensorReading {
            component: Component::Total,
            watts: self.total_watts(),
            confidence: if any_estimated {
                Confidence::Estimated
            } else {
                Confidence::Measured
            },
            timestamp: self.timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedSensor {
        reading: SensorReading,
    }

    impl PowerSensor for FixedSensor {
        fn sample(&mut self) -> Result<SensorReading, SensorError> {
            Ok(self.reading.clone())
        }
    }

    struct AlwaysFailsSensor;

    impl PowerSensor for AlwaysFailsSensor {
        fn sample(&mut self) -> Result<SensorReading, SensorError> {
            Err(SensorError::Unavailable("no hardware".to_string()))
        }
    }

    fn measured_reading(component: Component, watts: f64) -> SensorReading {
        SensorReading {
            component,
            watts,
            confidence: Confidence::Measured,
            timestamp: Utc::now(),
        }
    }

    fn estimated_reading(component: Component, watts: f64) -> SensorReading {
        SensorReading {
            component,
            watts,
            confidence: Confidence::Estimated,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn collects_readings_from_every_sensor() {
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(FixedSensor {
                reading: measured_reading(Component::Cpu, 10.0),
            }),
        );
        sampler.add_sensor(
            "ram",
            Box::new(FixedSensor {
                reading: estimated_reading(Component::Ram, 4.0),
            }),
        );

        let snapshot = sampler.sample_all();

        let watts: Vec<f64> = snapshot.readings().map(|r| r.watts).collect();
        assert_eq!(watts, vec![10.0, 4.0]);
    }

    #[test]
    fn a_failing_sensor_does_not_hide_readings_from_the_others() {
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(FixedSensor {
                reading: measured_reading(Component::Cpu, 10.0),
            }),
        );
        sampler.add_sensor("gpu", Box::new(AlwaysFailsSensor));

        let snapshot = sampler.sample_all();

        assert_eq!(snapshot.readings().count(), 1);
        assert_eq!(snapshot.results[1].0, "gpu");
        assert!(snapshot.results[1].1.is_err());
    }

    #[test]
    fn total_watts_sums_only_successful_readings() {
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(FixedSensor {
                reading: measured_reading(Component::Cpu, 10.0),
            }),
        );
        sampler.add_sensor(
            "ram",
            Box::new(FixedSensor {
                reading: estimated_reading(Component::Ram, 4.0),
            }),
        );
        sampler.add_sensor("gpu", Box::new(AlwaysFailsSensor));

        let snapshot = sampler.sample_all();

        assert_eq!(snapshot.total_watts(), 14.0);
    }

    #[test]
    fn total_is_measured_when_every_contributing_reading_is_measured() {
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(FixedSensor {
                reading: measured_reading(Component::Cpu, 10.0),
            }),
        );

        let snapshot = sampler.sample_all();

        assert_eq!(snapshot.total().unwrap().confidence, Confidence::Measured);
    }

    #[test]
    fn total_is_estimated_when_any_contributing_reading_is_estimated() {
        let mut sampler = Sampler::new();
        sampler.add_sensor(
            "cpu",
            Box::new(FixedSensor {
                reading: measured_reading(Component::Cpu, 10.0),
            }),
        );
        sampler.add_sensor(
            "ram",
            Box::new(FixedSensor {
                reading: estimated_reading(Component::Ram, 4.0),
            }),
        );

        let snapshot = sampler.sample_all();

        assert_eq!(snapshot.total().unwrap().confidence, Confidence::Estimated);
    }

    #[test]
    fn total_is_none_when_nothing_reported_successfully() {
        let mut sampler = Sampler::new();
        sampler.add_sensor("gpu", Box::new(AlwaysFailsSensor));

        let snapshot = sampler.sample_all();

        assert!(snapshot.total().is_none());
    }

    #[test]
    fn is_empty_reflects_whether_any_sensor_was_registered() {
        let mut sampler = Sampler::new();
        assert!(sampler.is_empty());

        sampler.add_sensor("cpu", Box::new(AlwaysFailsSensor));
        assert!(!sampler.is_empty());
    }
}
