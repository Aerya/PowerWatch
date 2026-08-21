use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use std::path::{Path, PathBuf};

pub const DEFAULT_WATTS_PER_GB: f64 = 0.3;

#[derive(Debug, PartialEq)]
pub struct MemInfo {
    pub total_kb: u64,
    pub available_kb: u64,
}

pub fn parse_meminfo(contents: &str) -> Result<MemInfo, SensorError> {
    let mut total_kb = None;
    let mut available_kb = None;

    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total_kb = Some(parse_kb_value(value)?);
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available_kb = Some(parse_kb_value(value)?);
        }
    }

    match (total_kb, available_kb) {
        (Some(total_kb), Some(available_kb)) => Ok(MemInfo {
            total_kb,
            available_kb,
        }),
        _ => Err(SensorError::ReadFailed(
            "missing MemTotal or MemAvailable in /proc/meminfo".to_string(),
        )),
    }
}

fn parse_kb_value(value: &str) -> Result<u64, SensorError> {
    value
        .trim()
        .trim_end_matches(" kB")
        .parse::<u64>()
        .map_err(|e| SensorError::ReadFailed(format!("invalid meminfo value: {e}")))
}

pub fn estimate_ram_watts(mem: &MemInfo, watts_per_gb: f64) -> f64 {
    let used_kb = mem.total_kb.saturating_sub(mem.available_kb);
    let used_gb = used_kb as f64 / (1024.0 * 1024.0);
    used_gb * watts_per_gb
}

pub struct RamSensor {
    meminfo_path: PathBuf,
    watts_per_gb: f64,
}

impl RamSensor {
    pub fn new(meminfo_path: PathBuf) -> Self {
        Self {
            meminfo_path,
            watts_per_gb: DEFAULT_WATTS_PER_GB,
        }
    }
}

impl PowerSensor for RamSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let contents = read_meminfo_file(&self.meminfo_path)?;
        let mem = parse_meminfo(&contents)?;

        Ok(SensorReading {
            component: Component::Ram,
            watts: estimate_ram_watts(&mem, self.watts_per_gb),
            confidence: Confidence::Estimated,
            timestamp: chrono::Utc::now(),
        })
    }
}

fn read_meminfo_file(path: &Path) -> Result<String, SensorError> {
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

    const SAMPLE_MEMINFO: &str = "\
MemTotal:        4093928 kB
MemFree:         3973176 kB
MemAvailable:    3885520 kB
Buffers:           11300 kB
Cached:            52928 kB
";

    #[test]
    fn parses_total_and_available_memory() {
        let mem = parse_meminfo(SAMPLE_MEMINFO).unwrap();

        assert_eq!(
            mem,
            MemInfo {
                total_kb: 4_093_928,
                available_kb: 3_885_520,
            }
        );
    }

    #[test]
    fn fails_when_required_fields_are_missing() {
        let result = parse_meminfo("SomeOtherField: 123 kB\n");

        assert!(matches!(result.unwrap_err(), SensorError::ReadFailed(_)));
    }

    #[test]
    fn estimates_more_watts_when_more_memory_is_used() {
        let mostly_free = MemInfo {
            total_kb: 16 * 1024 * 1024,
            available_kb: 15 * 1024 * 1024,
        };
        let mostly_used = MemInfo {
            total_kb: 16 * 1024 * 1024,
            available_kb: 2 * 1024 * 1024,
        };

        let watts_free = estimate_ram_watts(&mostly_free, DEFAULT_WATTS_PER_GB);
        let watts_used = estimate_ram_watts(&mostly_used, DEFAULT_WATTS_PER_GB);

        assert!(watts_used > watts_free);
    }

    #[test]
    fn sample_reports_an_estimated_ram_reading() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{SAMPLE_MEMINFO}").unwrap();
        let mut sensor = RamSensor::new(file.path().to_path_buf());

        let reading = sensor.sample().unwrap();

        assert_eq!(reading.component, Component::Ram);
        assert_eq!(reading.confidence, Confidence::Estimated);
        assert!(reading.watts > 0.0);
    }
}
