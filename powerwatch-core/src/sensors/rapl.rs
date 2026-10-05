use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn read_energy_uj(path: &Path) -> Result<u64, SensorError> {
    let raw = std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            SensorError::Unavailable(format!("{} does not exist", path.display()))
        }
        std::io::ErrorKind::PermissionDenied => SensorError::PermissionDenied(format!(
            "no read access to {} (see docs/setup-udev for a fix)",
            path.display()
        )),
        _ => SensorError::ReadFailed(e.to_string()),
    })?;

    raw.trim()
        .parse::<u64>()
        .map_err(|e| SensorError::ReadFailed(format!("invalid energy value: {e}")))
}

pub fn compute_power_watts(
    before_uj: u64,
    after_uj: u64,
    elapsed: Duration,
    max_energy_uj: u64,
) -> f64 {
    let delta_uj = energy_delta_uj(before_uj, after_uj, max_energy_uj);
    let delta_joules = delta_uj as f64 / 1_000_000.0;
    delta_joules / elapsed.as_secs_f64()
}

fn energy_delta_uj(before_uj: u64, after_uj: u64, max_energy_uj: u64) -> u64 {
    if after_uj >= before_uj {
        after_uj - before_uj
    } else {
        (max_energy_uj - before_uj) + after_uj
    }
}

pub struct RaplSensor {
    energy_path: PathBuf,
    max_energy_uj: u64,
    component: Component,
    last: Option<(Instant, u64)>,
}

impl RaplSensor {
    pub fn new(energy_path: PathBuf, max_energy_uj: u64) -> Self {
        Self::new_for_component(energy_path, max_energy_uj, Component::Cpu)
    }

    pub fn new_for_component(
        energy_path: PathBuf,
        max_energy_uj: u64,
        component: Component,
    ) -> Self {
        Self {
            energy_path,
            max_energy_uj,
            component,
            last: None,
        }
    }

    fn sample_at(&mut self, now: Instant) -> Result<SensorReading, SensorError> {
        let energy_uj = read_energy_uj(&self.energy_path)?;

        let Some((last_time, last_energy_uj)) = self.last else {
            self.last = Some((now, energy_uj));
            return Err(SensorError::NotReady(
                "need a second sample to compute power".to_string(),
            ));
        };

        let watts = compute_power_watts(
            last_energy_uj,
            energy_uj,
            now - last_time,
            self.max_energy_uj,
        );
        self.last = Some((now, energy_uj));

        Ok(SensorReading {
            component: self.component.clone(),
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

impl PowerSensor for RaplSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        self.sample_at(Instant::now())
    }
}

pub struct RaplPackageMinusDomainSensor {
    package_energy_path: PathBuf,
    package_max_energy_uj: u64,
    excluded_energy_path: PathBuf,
    excluded_max_energy_uj: u64,
    last: Option<(Instant, u64, u64)>,
}

impl RaplPackageMinusDomainSensor {
    pub fn new(
        package_energy_path: PathBuf,
        package_max_energy_uj: u64,
        excluded_energy_path: PathBuf,
        excluded_max_energy_uj: u64,
    ) -> Self {
        Self {
            package_energy_path,
            package_max_energy_uj,
            excluded_energy_path,
            excluded_max_energy_uj,
            last: None,
        }
    }

    fn sample_at(&mut self, now: Instant) -> Result<SensorReading, SensorError> {
        let package_energy_uj = read_energy_uj(&self.package_energy_path)?;
        let excluded_energy_uj = read_energy_uj(&self.excluded_energy_path)?;

        let Some((last_time, last_package_uj, last_excluded_uj)) = self.last else {
            self.last = Some((now, package_energy_uj, excluded_energy_uj));
            return Err(SensorError::NotReady(
                "need a second sample to compute power".to_string(),
            ));
        };

        let elapsed = now - last_time;
        if elapsed.is_zero() {
            return Err(SensorError::NotReady(
                "RAPL energy samples are too close together".to_string(),
            ));
        }
        let package_delta = energy_delta_uj(
            last_package_uj,
            package_energy_uj,
            self.package_max_energy_uj,
        );
        let excluded_delta = energy_delta_uj(
            last_excluded_uj,
            excluded_energy_uj,
            self.excluded_max_energy_uj,
        );
        self.last = Some((now, package_energy_uj, excluded_energy_uj));

        let cpu_delta = package_delta.saturating_sub(excluded_delta);
        let watts = (cpu_delta as f64 / 1_000_000.0) / elapsed.as_secs_f64();

        Ok(SensorReading {
            component: Component::Cpu,
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

impl PowerSensor for RaplPackageMinusDomainSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        self.sample_at(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_energy_file(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{contents}").unwrap();
        file
    }

    #[test]
    fn reads_energy_value_from_a_plain_text_file() {
        let file = write_energy_file("123456\n");

        let energy = read_energy_uj(file.path()).unwrap();

        assert_eq!(energy, 123456);
    }

    #[test]
    fn rejects_a_file_with_non_numeric_content() {
        let file = write_energy_file("not-a-number\n");

        let error = read_energy_uj(file.path()).unwrap_err();

        assert!(matches!(error, SensorError::ReadFailed(_)));
    }

    #[test]
    fn reports_unavailable_when_the_sysfs_path_is_missing() {
        let missing = Path::new("/tmp/powerwatch-test-path-that-does-not-exist/energy_uj");

        let error = read_energy_uj(missing).unwrap_err();

        assert!(matches!(error, SensorError::Unavailable(_)));
    }

    #[test]
    fn computes_watts_from_energy_delta_over_time() {
        let watts = compute_power_watts(1_000_000, 3_000_000, Duration::from_secs(1), u64::MAX);

        assert_eq!(watts, 2.0);
    }

    #[test]
    fn handles_the_rapl_counter_wrapping_around() {
        let max = 10_000_000u64;
        let watts = compute_power_watts(max - 100, 50, Duration::from_secs(1), max);

        assert_eq!(watts, 150.0 / 1_000_000.0);
    }

    #[test]
    fn first_sample_returns_not_ready_since_theres_no_baseline_yet() {
        let file = write_energy_file("1000000\n");
        let mut sensor = RaplSensor::new(file.path().to_path_buf(), u64::MAX);

        let result = sensor.sample_at(Instant::now());

        assert!(matches!(result.unwrap_err(), SensorError::NotReady(_)));
    }

    #[test]
    fn second_sample_reports_measured_power_for_the_cpu() {
        let file = write_energy_file("1000000\n");
        let mut sensor = RaplSensor::new(file.path().to_path_buf(), u64::MAX);
        let t0 = Instant::now();
        sensor.sample_at(t0).unwrap_err(); // warms up the baseline

        std::fs::write(file.path(), "3000000\n").unwrap();
        let reading = sensor.sample_at(t0 + Duration::from_secs(1)).unwrap();

        assert_eq!(reading.component, Component::Cpu);
        assert_eq!(reading.confidence, Confidence::Measured);
        assert_eq!(reading.watts, 2.0);
    }

    #[test]
    fn package_minus_uncore_avoids_double_counting_the_igpu() {
        let package = write_energy_file("1000000\n");
        let uncore = write_energy_file("200000\n");
        let mut sensor = RaplPackageMinusDomainSensor::new(
            package.path().to_path_buf(),
            u64::MAX,
            uncore.path().to_path_buf(),
            u64::MAX,
        );
        let t0 = Instant::now();
        sensor.sample_at(t0).unwrap_err();

        std::fs::write(package.path(), "4000000\n").unwrap();
        std::fs::write(uncore.path(), "1200000\n").unwrap();
        let reading = sensor.sample_at(t0 + Duration::from_secs(1)).unwrap();

        assert_eq!(reading.component, Component::Cpu);
        assert_eq!(reading.watts, 2.0);
    }
}
