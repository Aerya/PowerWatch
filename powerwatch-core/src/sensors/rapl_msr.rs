use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

const MSR_RAPL_POWER_UNIT: u64 = 0x606;
const MSR_PKG_ENERGY_STATUS: u64 = 0x611;
const ENERGY_STATUS_MASK: u64 = 0xffff_ffff;

fn map_open_error(path: &Path, error: std::io::Error) -> SensorError {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            SensorError::Unavailable(format!("{} does not exist", path.display()))
        }
        std::io::ErrorKind::PermissionDenied => SensorError::PermissionDenied(format!(
            "no read access to {} (map /dev/cpu/0/msr into the container)",
            path.display()
        )),
        _ => SensorError::ReadFailed(error.to_string()),
    }
}

fn read_msr_u64(file: &mut File, register: u64) -> Result<u64, SensorError> {
    file.seek(SeekFrom::Start(register))
        .map_err(|error| SensorError::ReadFailed(format!("MSR seek 0x{register:x}: {error}")))?;

    let mut bytes = [0u8; 8];
    file.read_exact(&mut bytes)
        .map_err(|error| SensorError::ReadFailed(format!("MSR read 0x{register:x}: {error}")))?;

    Ok(u64::from_le_bytes(bytes))
}

fn rapl_energy_unit_joules(power_unit_msr: u64) -> f64 {
    let exponent = ((power_unit_msr >> 8) & 0x1f) as i32;
    2f64.powi(-exponent)
}

fn compute_msr_power_watts(
    before_raw: u32,
    after_raw: u32,
    elapsed: Duration,
    energy_unit_joules: f64,
) -> f64 {
    let delta_raw = after_raw.wrapping_sub(before_raw) as f64;
    (delta_raw * energy_unit_joules) / elapsed.as_secs_f64()
}

pub struct RaplMsrSensor {
    file: File,
    energy_unit_joules: f64,
    last: Option<(Instant, u32)>,
}

impl RaplMsrSensor {
    pub fn new(path: &Path) -> Result<Self, SensorError> {
        let mut file = File::open(path).map_err(|error| map_open_error(path, error))?;
        let power_unit_msr = read_msr_u64(&mut file, MSR_RAPL_POWER_UNIT)?;
        let energy_unit_joules = rapl_energy_unit_joules(power_unit_msr);

        let _ = read_msr_u64(&mut file, MSR_PKG_ENERGY_STATUS)?;

        Ok(Self {
            file,
            energy_unit_joules,
            last: None,
        })
    }

    fn sample_at(&mut self, now: Instant) -> Result<SensorReading, SensorError> {
        let raw = (read_msr_u64(&mut self.file, MSR_PKG_ENERGY_STATUS)? & ENERGY_STATUS_MASK) as u32;

        let Some((last_time, last_raw)) = self.last else {
            self.last = Some((now, raw));
            return Err(SensorError::NotReady(
                "need a second MSR sample to compute power".to_string(),
            ));
        };

        let elapsed = now - last_time;
        if elapsed.is_zero() {
            return Err(SensorError::NotReady(
                "RAPL MSR energy samples are too close together".to_string(),
            ));
        }

        let watts =
            compute_msr_power_watts(last_raw, raw, elapsed, self.energy_unit_joules);
        self.last = Some((now, raw));

        Ok(SensorReading {
            component: Component::Cpu,
            watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

impl PowerSensor for RaplMsrSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        self.sample_at(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_register(file: &mut tempfile::NamedTempFile, register: u64, value: u64) {
        file.as_file_mut().seek(SeekFrom::Start(register)).unwrap();
        file.as_file_mut().write_all(&value.to_le_bytes()).unwrap();
        file.as_file_mut().flush().unwrap();
    }

    fn fake_msr_file(power_unit: u64, package_energy: u32) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write_register(&mut file, MSR_RAPL_POWER_UNIT, power_unit);
        write_register(
            &mut file,
            MSR_PKG_ENERGY_STATUS,
            u64::from(package_energy),
        );
        file
    }

    #[test]
    fn decodes_goldmont_plus_energy_unit() {
        let unit = rapl_energy_unit_joules(0x330a0e08);

        assert!((unit - (1.0 / 16384.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn computes_measured_package_power_from_msr_delta() {
        let mut file = fake_msr_file(14u64 << 8, 100);
        let mut sensor = RaplMsrSensor::new(file.path()).unwrap();
        let t0 = Instant::now();

        sensor.sample_at(t0).unwrap_err();

        write_register(&mut file, MSR_PKG_ENERGY_STATUS, 100 + 16384);
        let reading = sensor.sample_at(t0 + Duration::from_secs(1)).unwrap();

        assert_eq!(reading.component, Component::Cpu);
        assert_eq!(reading.confidence, Confidence::Measured);
        assert!((reading.watts - 1.0).abs() < 1e-9);
    }

    #[test]
    fn handles_the_32_bit_package_energy_counter_wrapping() {
        let mut file = fake_msr_file(0, u32::MAX - 10);
        let mut sensor = RaplMsrSensor::new(file.path()).unwrap();
        let t0 = Instant::now();

        sensor.sample_at(t0).unwrap_err();

        write_register(&mut file, MSR_PKG_ENERGY_STATUS, 5);
        let reading = sensor.sample_at(t0 + Duration::from_secs(1)).unwrap();

        assert_eq!(reading.watts, 16.0);
    }

    #[test]
    fn rejects_a_missing_msr_device() {
        let result = RaplMsrSensor::new(Path::new(
            "/tmp/powerwatch-msr-device-does-not-exist",
        ));

        assert!(matches!(result, Err(SensorError::Unavailable(_))));
    }
}
