use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Component {
    Cpu,
    Gpu(GpuVendor),
    Ram,
    Disk(String),
    Total,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Measured,
    Estimated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorReading {
    pub component: Component,
    pub watts: f64,
    pub confidence: Confidence,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorError {
    PermissionDenied(String),
    Unavailable(String),
    ReadFailed(String),
    NotReady(String),
}

pub trait PowerSensor: Send {
    fn sample(&mut self) -> Result<SensorReading, SensorError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn reading_round_trips_through_json() {
        let reading = SensorReading {
            component: Component::Cpu,
            watts: 12.5,
            confidence: Confidence::Measured,
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&reading).unwrap();
        let parsed: SensorReading = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed, reading);
    }

    #[test]
    fn estimated_readings_are_marked_as_such() {
        let reading = SensorReading {
            component: Component::Ram,
            watts: 4.0,
            confidence: Confidence::Estimated,
            timestamp: Utc::now(),
        };

        assert_eq!(reading.confidence, Confidence::Estimated);
    }

    struct FakeSensor {
        next_reading: SensorReading,
    }

    impl PowerSensor for FakeSensor {
        fn sample(&mut self) -> Result<SensorReading, SensorError> {
            Ok(self.next_reading.clone())
        }
    }

    #[test]
    fn a_sensor_can_be_used_through_the_trait() {
        let reading = SensorReading {
            component: Component::Gpu(GpuVendor::Nvidia),
            watts: 80.0,
            confidence: Confidence::Measured,
            timestamp: Utc::now(),
        };
        let mut sensor: Box<dyn PowerSensor> = Box::new(FakeSensor {
            next_reading: reading.clone(),
        });

        let result = sensor.sample().unwrap();

        assert_eq!(result, reading);
    }

    #[test]
    fn sensor_errors_can_report_why_a_reading_failed() {
        struct BrokenSensor;

        impl PowerSensor for BrokenSensor {
            fn sample(&mut self) -> Result<SensorReading, SensorError> {
                Err(SensorError::PermissionDenied(
                    "no read access to /sys/class/powercap".to_string(),
                ))
            }
        }

        let mut sensor = BrokenSensor;

        let error = sensor.sample().unwrap_err();

        assert_eq!(
            error,
            SensorError::PermissionDenied("no read access to /sys/class/powercap".to_string())
        );
    }
}
