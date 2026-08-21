use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use crate::sensors::disk::DiskType;

pub trait DiskActivitySource: Send {
    fn read(&mut self) -> Result<f64, SensorError>;
}

pub fn parse_iostat_tps(output: &str) -> Result<f64, SensorError> {
    let numeric_lines: Vec<&str> = output
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && trimmed
                    .split_whitespace()
                    .all(|tok| tok.parse::<f64>().is_ok())
        })
        .collect();

    let last_line = numeric_lines.last().ok_or_else(|| {
        SensorError::ReadFailed("no numeric data lines found in iostat output".to_string())
    })?;

    let fields: Vec<&str> = last_line.split_whitespace().collect();
    let tps_field = fields.get(1).ok_or_else(|| {
        SensorError::ReadFailed("iostat data line had fewer fields than expected".to_string())
    })?;

    tps_field
        .parse::<f64>()
        .map_err(|e| SensorError::ReadFailed(format!("invalid tps value: {e}")))
}

pub struct MacDiskSensor<S: DiskActivitySource> {
    source: S,
    disk_type: DiskType,
}

impl<S: DiskActivitySource> MacDiskSensor<S> {
    pub fn new(source: S, disk_type: DiskType) -> Self {
        Self { source, disk_type }
    }
}

impl<S: DiskActivitySource> PowerSensor for MacDiskSensor<S> {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let tps = self.source.read()?;

        let watts = if tps > 0.0 {
            self.disk_type.active_watts()
        } else {
            self.disk_type.idle_watts()
        };

        Ok(SensorReading {
            component: Component::Disk("macOS".to_string()),
            watts,
            confidence: Confidence::Estimated,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::{parse_iostat_tps, DiskActivitySource};
    use crate::model::SensorError;
    use std::process::Command;

    pub struct RealDiskActivitySource;

    impl DiskActivitySource for RealDiskActivitySource {
        fn read(&mut self) -> Result<f64, SensorError> {
            let output = Command::new("iostat")
                .args(["-d", "-c", "2", "-w", "1", "disk0"])
                .output()
                .map_err(|e| SensorError::ReadFailed(format!("failed to run iostat: {e}")))?;

            if !output.status.success() {
                return Err(SensorError::ReadFailed(format!(
                    "iostat exited with an error: {}",
                    String::from_utf8_lossy(&output.stderr)
                )));
            }

            let text = String::from_utf8(output.stdout).map_err(|e| {
                SensorError::ReadFailed(format!("iostat output wasn't valid UTF-8: {e}"))
            })?;

            parse_iostat_tps(&text)
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos_impl::RealDiskActivitySource;

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_IOSTAT_ACTIVE: &str =
        "          disk0\n   KB/t tps  MB/s\n  40.58  43   1.69\n  52.10  67   2.30\n";
    const SAMPLE_IOSTAT_IDLE: &str =
        "          disk0\n   KB/t tps  MB/s\n  40.58  43   1.69\n   0.00   0   0.00\n";

    #[test]
    fn extracts_tps_from_the_last_data_line() {
        let tps = parse_iostat_tps(SAMPLE_IOSTAT_ACTIVE).unwrap();

        assert_eq!(tps, 67.0);
    }

    #[test]
    fn extracts_zero_tps_when_idle() {
        let tps = parse_iostat_tps(SAMPLE_IOSTAT_IDLE).unwrap();

        assert_eq!(tps, 0.0);
    }

    #[test]
    fn fails_clearly_when_thereas_no_numeric_data() {
        let result = parse_iostat_tps("          disk0\n   KB/t tps  MB/s\n");

        assert!(matches!(result.unwrap_err(), SensorError::ReadFailed(_)));
    }

    struct FakeDiskActivitySource {
        tps: f64,
    }

    impl DiskActivitySource for FakeDiskActivitySource {
        fn read(&mut self) -> Result<f64, SensorError> {
            Ok(self.tps)
        }
    }

    #[test]
    fn reports_active_watts_when_tps_is_above_zero() {
        let source = FakeDiskActivitySource { tps: 67.0 };
        let mut sensor = MacDiskSensor::new(source, DiskType::SsdSata);

        let reading = sensor.sample().unwrap();

        assert_eq!(reading.watts, DiskType::SsdSata.active_watts());
        assert_eq!(reading.confidence, Confidence::Estimated);
    }

    #[test]
    fn reports_idle_watts_when_tps_is_zero() {
        let source = FakeDiskActivitySource { tps: 0.0 };
        let mut sensor = MacDiskSensor::new(source, DiskType::Nvme);

        let reading = sensor.sample().unwrap();

        assert_eq!(reading.watts, DiskType::Nvme.idle_watts());
    }
}
