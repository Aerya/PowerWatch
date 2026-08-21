use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};

pub trait LhmPowerSource: Send {
    fn read_cpu_package_watts(&mut self) -> Result<f64, SensorError>;
}

pub struct LibreHardwareMonitorCpuSensor<S: LhmPowerSource> {
    source: S,
}

impl<S: LhmPowerSource> LibreHardwareMonitorCpuSensor<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }
}

impl<S: LhmPowerSource> PowerSensor for LibreHardwareMonitorCpuSensor<S> {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let watts = self.source.read_cpu_package_watts()?;

        Ok(SensorReading {
            component: Component::Cpu,
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(target_os = "windows")]
mod win32 {
    use super::LhmPowerSource;
    use crate::model::SensorError;
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct SensorRow {
        value: f32,
    }

    pub struct RealLhmPowerSource {
        conn: WMIConnection,
    }

    impl RealLhmPowerSource {
        pub fn new() -> Result<Self, SensorError> {
            let com = COMLibrary::new()
                .map_err(|e| SensorError::Unavailable(format!("COM initialization failed: {e}")))?;
            let conn = WMIConnection::with_namespace_path("ROOT\\LibreHardwareMonitor", com)
                .map_err(|e| {
                    SensorError::Unavailable(format!(
                        "LibreHardwareMonitor WMI namespace not found - is it running? ({e})"
                    ))
                })?;

            Ok(Self { conn })
        }
    }

    impl LhmPowerSource for RealLhmPowerSource {
        fn read_cpu_package_watts(&mut self) -> Result<f64, SensorError> {
            let results: Vec<SensorRow> = self
                .conn
                .raw_query(
                    "SELECT Value FROM Sensor WHERE SensorType = 'Power' AND Name LIKE '%CPU Package%'",
                )
                .map_err(|e| SensorError::ReadFailed(format!("WMI query failed: {e}")))?;

            results
                .into_iter()
                .next()
                .map(|row| row.value as f64)
                .ok_or_else(|| {
                    SensorError::Unavailable(
                        "no 'CPU Package' power sensor found in LibreHardwareMonitor".to_string(),
                    )
                })
        }
    }
}

#[cfg(target_os = "windows")]
pub use win32::RealLhmPowerSource;

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeLhmPowerSource {
        watts: f64,
    }

    impl LhmPowerSource for FakeLhmPowerSource {
        fn read_cpu_package_watts(&mut self) -> Result<f64, SensorError> {
            Ok(self.watts)
        }
    }

    #[test]
    fn reports_a_measured_reading_from_the_source() {
        let source = FakeLhmPowerSource { watts: 42.5 };
        let mut sensor = LibreHardwareMonitorCpuSensor::new(source);

        let reading = sensor.sample().unwrap();

        assert_eq!(reading.component, Component::Cpu);
        assert_eq!(reading.confidence, Confidence::Measured);
        assert_eq!(reading.watts, 42.5);
    }

    struct FailingLhmPowerSource;

    impl LhmPowerSource for FailingLhmPowerSource {
        fn read_cpu_package_watts(&mut self) -> Result<f64, SensorError> {
            Err(SensorError::Unavailable(
                "LibreHardwareMonitor not running".to_string(),
            ))
        }
    }

    #[test]
    fn propagates_source_errors_without_panicking() {
        let mut sensor = LibreHardwareMonitorCpuSensor::new(FailingLhmPowerSource);

        let result = sensor.sample();

        assert!(matches!(result.unwrap_err(), SensorError::Unavailable(_)));
    }
}
