use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskType {
    Hdd7200Rpm,
    SsdSata,
    Nvme,
}

impl DiskType {
    pub fn idle_watts(self) -> f64 {
        match self {
            DiskType::Hdd7200Rpm => 4.0,
            DiskType::SsdSata => 0.5,
            DiskType::Nvme => 1.0,
        }
    }

    pub fn active_watts(self) -> f64 {
        match self {
            DiskType::Hdd7200Rpm => 8.0,
            DiskType::SsdSata => 3.0,
            DiskType::Nvme => 6.0,
        }
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

pub struct DiskSensor {
    diskstats_path: PathBuf,
    device_name: String,
    disk_type: DiskType,
    last_io_count: Option<u64>,
}

impl DiskSensor {
    pub fn new(diskstats_path: PathBuf, device_name: String, disk_type: DiskType) -> Self {
        Self {
            diskstats_path,
            device_name,
            disk_type,
            last_io_count: None,
        }
    }

    fn sample_at(&mut self, _now: Instant) -> Result<SensorReading, SensorError> {
        let contents = read_diskstats_file(&self.diskstats_path)?;
        let io_count = read_io_count(&contents, &self.device_name)?;
        let was_active = self.last_io_count.is_some_and(|last| io_count != last);
        self.last_io_count = Some(io_count);

        let watts = if was_active {
            self.disk_type.active_watts()
        } else {
            self.disk_type.idle_watts()
        };

        Ok(SensorReading {
            component: Component::Disk(self.device_name.clone()),
            watts,
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

    fn sample_diskstats(sda_reads: u64, sda_writes: u64) -> String {
        format!(
            "   7       0 loop0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n\
               8       0 sda {sda_reads} 0 234567 6789 {sda_writes} 0 123456 4567 0 3456 8901 0 0 0 0 0\n"
        )
    }

    #[test]
    fn sums_reads_and_writes_for_the_requested_device() {
        let contents = sample_diskstats(100, 50);

        let io_count = read_io_count(&contents, "sda").unwrap();

        assert_eq!(io_count, 150);
    }

    #[test]
    fn reports_unavailable_for_a_device_not_in_diskstats() {
        let contents = sample_diskstats(100, 50);

        let error = read_io_count(&contents, "nvme0n1").unwrap_err();

        assert!(matches!(error, SensorError::Unavailable(_)));
    }

    #[test]
    fn first_sample_assumes_idle_since_theres_nothing_to_compare_to() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}", sample_diskstats(100, 50)).unwrap();
        let mut sensor =
            DiskSensor::new(file.path().to_path_buf(), "sda".to_string(), DiskType::Nvme);

        let reading = sensor.sample_at(Instant::now()).unwrap();

        assert_eq!(reading.watts, DiskType::Nvme.idle_watts());
        assert_eq!(reading.confidence, Confidence::Estimated);
    }

    #[test]
    fn reports_active_watts_when_io_counters_changed_between_samples() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}", sample_diskstats(100, 50)).unwrap();
        let mut sensor = DiskSensor::new(
            file.path().to_path_buf(),
            "sda".to_string(),
            DiskType::SsdSata,
        );
        sensor.sample_at(Instant::now()).unwrap();

        std::fs::write(file.path(), sample_diskstats(200, 90)).unwrap();
        let reading = sensor.sample_at(Instant::now()).unwrap();

        assert_eq!(reading.watts, DiskType::SsdSata.active_watts());
    }

    #[test]
    fn reports_idle_watts_when_io_counters_stayed_the_same() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{}", sample_diskstats(100, 50)).unwrap();
        let mut sensor = DiskSensor::new(
            file.path().to_path_buf(),
            "sda".to_string(),
            DiskType::Hdd7200Rpm,
        );
        sensor.sample_at(Instant::now()).unwrap();

        let reading = sensor.sample_at(Instant::now()).unwrap();

        assert_eq!(reading.watts, DiskType::Hdd7200Rpm.idle_watts());
    }
}
