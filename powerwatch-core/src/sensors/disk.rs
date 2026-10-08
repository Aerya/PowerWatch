use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskType {
    Hdd7200Rpm,
    SsdSata,
    LowPowerFlash,
    Nvme,
}

impl DiskType {
    pub fn idle_watts(self) -> f64 {
        match self {
            DiskType::Hdd7200Rpm => 4.0,
            DiskType::SsdSata => 0.5,
            DiskType::LowPowerFlash => 0.2,
            DiskType::Nvme => 1.0,
        }
    }

    pub fn active_watts(self) -> f64 {
        match self {
            DiskType::Hdd7200Rpm => 8.0,
            DiskType::SsdSata => 3.0,
            DiskType::LowPowerFlash => 1.5,
            DiskType::Nvme => 6.0,
        }
    }

    pub fn estimated_watts(self, busy_ratio: f64) -> f64 {
        let ratio = busy_ratio.clamp(0.0, 1.0);
        self.idle_watts() + (self.active_watts() - self.idle_watts()) * ratio
    }
}

pub fn read_io_count(contents: &str, device_name: &str) -> Result<u64, SensorError> {
    for line in contents.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 || fields[2] != device_name {
            continue;
        }

        let reads_completed: u64 = fields[3]
            .parse()
            .map_err(|e| SensorError::ReadFailed(format!("invalid diskstats field: {e}")))?;
        let writes_completed: u64 = fields[7]
            .parse()
            .map_err(|e| SensorError::ReadFailed(format!("invalid diskstats field: {e}")))?;

        return Ok(reads_completed + writes_completed);
    }

    Err(SensorError::Unavailable(format!(
        "device {device_name} not found in diskstats"
    )))
}

pub fn read_io_busy_ms(contents: &str, device_name: &str) -> Result<u64, SensorError> {
    for line in contents.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 13 || fields[2] != device_name {
            continue;
        }

        return fields[12]
            .parse()
            .map_err(|e| SensorError::ReadFailed(format!("invalid diskstats busy-time field: {e}")));
    }

    Err(SensorError::Unavailable(format!(
        "device {device_name} not found in diskstats"
    )))
}

pub struct DiskSensor {
    diskstats_path: PathBuf,
    device_name: String,
    disk_type: DiskType,
    last_busy_ms: Option<u64>,
    last_sample_at: Option<Instant>,
}

impl DiskSensor {
    pub fn new(diskstats_path: PathBuf, device_name: String, disk_type: DiskType) -> Self {
        Self {
            diskstats_path,
            device_name,
            disk_type,
            last_busy_ms: None,
            last_sample_at: None,
        }
    }

    fn sample_at(&mut self, now: Instant) -> Result<SensorReading, SensorError> {
        let contents = read_diskstats_file(&self.diskstats_path)?;
        let busy_ms = read_io_busy_ms(&contents, &self.device_name)?;

        let busy_ratio = match (self.last_busy_ms, self.last_sample_at) {
            (Some(previous_busy_ms), Some(previous_sample_at))
                if busy_ms >= previous_busy_ms && now > previous_sample_at =>
            {
                let busy_delta_ms = (busy_ms - previous_busy_ms) as f64;
                let elapsed_ms = now.duration_since(previous_sample_at).as_secs_f64() * 1000.0;
                if elapsed_ms > 0.0 {
                    (busy_delta_ms / elapsed_ms).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };

        self.last_busy_ms = Some(busy_ms);
        self.last_sample_at = Some(now);

        Ok(SensorReading {
            component: Component::Disk(self.device_name.clone()),
            watts: self.disk_type.estimated_watts(busy_ratio),
            confidence: Confidence::Estimated,
            timestamp: chrono::Utc::now(),
        })
    }
}

impl PowerSensor for DiskSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        self.sample_at(Instant::now())
    }
}

fn read_diskstats_file(path: &Path) -> Result<String, SensorError> {
    std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            SensorError::Unavailable(format!("{} does not exist", path.display()))
        }
        _ => SensorError::ReadFailed(e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    fn sample_diskstats(reads: u64, writes: u64, busy_ms: u64) -> String {
        format!(
            "   7       0 loop0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n\
               8       0 sda {reads} 0 234567 6789 {writes} 0 123456 4567 0 {busy_ms} 8901 0 0 0 0 0\n"
        )
    }

    #[test]
    fn sums_reads_and_writes_for_the_requested_device() {
        let contents = sample_diskstats(100, 50, 250);

        let io_count = read_io_count(&contents, "sda").unwrap();

        assert_eq!(io_count, 150);
    }

    #[test]
    fn reads_busy_time_for_the_requested_device() {
        let contents = sample_diskstats(100, 50, 250);

        let busy_ms = read_io_busy_ms(&contents, "sda").unwrap();

        assert_eq!(busy_ms, 250);
    }

    #[test]
    fn reports_unavailable_for_a_device_not_in_diskstats() {
        let contents = sample_diskstats(100, 50, 250);

        let error = read_io_busy_ms(&contents, "nvme0n1").unwrap_err();

        assert!(matches!(error, SensorError::Unavailable(_)));
    }

    #[test]
    fn first_sample_uses_the_idle_baseline() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}", sample_diskstats(100, 50, 250)).unwrap();
        let mut sensor =
            DiskSensor::new(file.path().to_path_buf(), "sda".to_string(), DiskType::Nvme);

        let reading = sensor.sample_at(Instant::now()).unwrap();

        assert_eq!(reading.watts, DiskType::Nvme.idle_watts());
        assert_eq!(reading.confidence, Confidence::Estimated);
    }

    #[test]
    fn interpolates_between_idle_and_active_from_busy_time() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}", sample_diskstats(100, 50, 1000)).unwrap();
        let mut sensor = DiskSensor::new(
            file.path().to_path_buf(),
            "sda".to_string(),
            DiskType::SsdSata,
        );
        let started = Instant::now();
        sensor.sample_at(started).unwrap();

        std::fs::write(file.path(), sample_diskstats(200, 90, 1500)).unwrap();
        let reading = sensor.sample_at(started + Duration::from_secs(1)).unwrap();

        assert!((reading.watts - 1.75).abs() < 0.0001);
    }

    #[test]
    fn clamps_busy_time_to_the_active_ceiling() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}", sample_diskstats(100, 50, 1000)).unwrap();
        let mut sensor = DiskSensor::new(
            file.path().to_path_buf(),
            "sda".to_string(),
            DiskType::Hdd7200Rpm,
        );
        let started = Instant::now();
        sensor.sample_at(started).unwrap();

        std::fs::write(file.path(), sample_diskstats(200, 90, 2500)).unwrap();
        let reading = sensor.sample_at(started + Duration::from_secs(1)).unwrap();

        assert_eq!(reading.watts, DiskType::Hdd7200Rpm.active_watts());
    }

    #[test]
    fn keeps_idle_baseline_when_busy_counter_does_not_move() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}", sample_diskstats(100, 50, 1000)).unwrap();
        let mut sensor = DiskSensor::new(
            file.path().to_path_buf(),
            "sda".to_string(),
            DiskType::LowPowerFlash,
        );
        let started = Instant::now();
        sensor.sample_at(started).unwrap();

        let reading = sensor.sample_at(started + Duration::from_secs(1)).unwrap();

        assert_eq!(reading.watts, DiskType::LowPowerFlash.idle_watts());
    }

    #[test]
    fn low_power_flash_uses_a_lower_estimation_profile() {
        assert_eq!(DiskType::LowPowerFlash.idle_watts(), 0.2);
        assert_eq!(DiskType::LowPowerFlash.active_watts(), 1.5);
    }
}
