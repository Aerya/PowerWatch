use crate::discovery::{GpuPowerSource, LinuxGpuTarget};
use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use std::path::Path;
use std::time::Instant;

fn read_counter(path: &Path) -> Result<u64, SensorError> {
    let raw = std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            SensorError::Unavailable(format!("{} does not exist", path.display()))
        }
        std::io::ErrorKind::PermissionDenied => {
            SensorError::PermissionDenied(format!("no read access to {}", path.display()))
        }
        _ => SensorError::ReadFailed(e.to_string()),
    })?;

    raw.trim()
        .parse::<u64>()
        .map_err(|e| SensorError::ReadFailed(format!("invalid GPU telemetry value: {e}")))
}

pub struct LinuxGpuPowerSensor {
    target: LinuxGpuTarget,
    last_energy: Option<(Instant, u64)>,
}

impl LinuxGpuPowerSensor {
    pub fn new(target: LinuxGpuTarget) -> Self {
        Self {
            target,
            last_energy: None,
        }
    }

    fn component(&self) -> Component {
        Component::GpuDevice {
            vendor: self.target.vendor.clone(),
            index: self.target.index,
            name: self.target.name.clone(),
        }
    }
}

impl PowerSensor for LinuxGpuPowerSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let watts = match &self.target.source {
            GpuPowerSource::PowerUw(path) => read_counter(path)? as f64 / 1_000_000.0,
            GpuPowerSource::EnergyUj(path) => {
                let now = Instant::now();
                let energy_uj = read_counter(path)?;
                let Some((previous_time, previous_energy)) = self.last_energy else {
                    self.last_energy = Some((now, energy_uj));
                    return Err(SensorError::NotReady(
                        "need a second GPU energy sample to compute power".to_string(),
                    ));
                };

                let elapsed = now.duration_since(previous_time).as_secs_f64();
                self.last_energy = Some((now, energy_uj));
                if elapsed <= f64::EPSILON {
                    return Err(SensorError::NotReady(
                        "GPU energy samples are too close together".to_string(),
                    ));
                }

                let delta_uj = if energy_uj >= previous_energy {
                    energy_uj - previous_energy
                } else {
                    (u64::MAX - previous_energy) + energy_uj + 1
                };
                (delta_uj as f64 / 1_000_000.0) / elapsed
            }
        };

        Ok(SensorReading {
            component: self.component(),
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::GpuPowerSource;
    use crate::model::GpuVendor;
    use std::io::Write;

    fn file_with(value: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{value}").unwrap();
        file
    }

    #[test]
    fn direct_hwmon_power_is_reported_in_watts() {
        let file = file_with("45000000\n");
        let target = LinuxGpuTarget {
            vendor: GpuVendor::Amd,
            index: 1,
            name: "AMD test GPU".to_string(),
            source: GpuPowerSource::PowerUw(file.path().to_path_buf()),
        };
        let mut sensor = LinuxGpuPowerSensor::new(target);
        let reading = sensor.sample().unwrap();

        assert_eq!(reading.watts, 45.0);
        assert_eq!(
            reading.component,
            Component::GpuDevice {
                vendor: GpuVendor::Amd,
                index: 1,
                name: "AMD test GPU".to_string(),
            }
        );
    }

    #[test]
    fn energy_counter_requires_a_baseline_then_reports_power() {
        let file = file_with("1000000\n");
        let target = LinuxGpuTarget {
            vendor: GpuVendor::Intel,
            index: 0,
            name: "Intel test GPU".to_string(),
            source: GpuPowerSource::EnergyUj(file.path().to_path_buf()),
        };
        let mut sensor = LinuxGpuPowerSensor::new(target);

        assert!(matches!(sensor.sample(), Err(SensorError::NotReady(_))));
        std::fs::write(file.path(), "2000000\n").unwrap();
        let reading = sensor.sample().unwrap();
        assert!(reading.watts >= 0.0);
    }
}
