use crate::model::{Component, Confidence, GpuVendor, PowerSensor, SensorError, SensorReading};
use std::path::{Path, PathBuf};

pub fn read_power_uw(path: &Path) -> Result<u64, SensorError> {
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
        .map_err(|e| SensorError::ReadFailed(format!("invalid power value: {e}")))
}

pub struct AmdGpuSensor {
    power_path: PathBuf,
}

impl AmdGpuSensor {
    pub fn new(power_path: PathBuf) -> Self {
        Self { power_path }
    }
}

impl PowerSensor for AmdGpuSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let power_uw = read_power_uw(&self.power_path)?;
        let watts = power_uw as f64 / 1_000_000.0;

        Ok(SensorReading {
            component: Component::Gpu(GpuVendor::Amd),
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_power_file(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{contents}").unwrap();
        file
    }

    #[test]
    fn reads_power_value_from_a_plain_text_file() {
        let file = write_power_file("45000000\n");

        let power = read_power_uw(file.path()).unwrap();

        assert_eq!(power, 45_000_000);
    }

    #[test]
    fn reports_unavailable_when_the_hwmon_path_is_missing() {
        let missing = Path::new("/tmp/powerwatch-test-amd-path-missing/power1_average");

        let error = read_power_uw(missing).unwrap_err();

        assert!(matches!(error, SensorError::Unavailable(_)));
    }

    #[test]
    fn sample_converts_microwatts_to_watts_and_marks_the_gpu_as_measured() {
        let file = write_power_file("45000000\n");
        let mut sensor = AmdGpuSensor::new(file.path().to_path_buf());

        let reading = sensor.sample().unwrap();

        assert_eq!(reading.component, Component::Gpu(GpuVendor::Amd));
        assert_eq!(reading.confidence, Confidence::Measured);
        assert_eq!(reading.watts, 45.0);
    }
}
