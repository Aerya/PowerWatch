use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use crate::sensors::ram::DEFAULT_WATTS_PER_GB;

#[derive(Debug, Clone, Copy)]
pub struct VmStat {
    pub page_size_bytes: u64,
    pub pages_free: u64,
    pub pages_inactive: u64,
    pub pages_speculative: u64,
}

pub fn parse_vm_stat(text: &str) -> Result<VmStat, SensorError> {
    let page_size_bytes = text
        .split("page size of ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .ok_or_else(|| {
            SensorError::ReadFailed("could not find page size in vm_stat output".to_string())
        })?
        .parse::<u64>()
        .map_err(|e| SensorError::ReadFailed(format!("invalid page size: {e}")))?;

    let pages_free = parse_page_count(text, "Pages free:")?;
    let pages_inactive = parse_page_count(text, "Pages inactive:")?;
    let pages_speculative = parse_page_count(text, "Pages speculative:")?;

    Ok(VmStat {
        page_size_bytes,
        pages_free,
        pages_inactive,
        pages_speculative,
    })
}

fn parse_page_count(text: &str, label: &str) -> Result<u64, SensorError> {
    let after_label = text
        .split(label)
        .nth(1)
        .ok_or_else(|| SensorError::ReadFailed(format!("'{label}' not found in vm_stat output")))?;

    let number_text = after_label
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .ok_or_else(|| SensorError::ReadFailed(format!("no number found after '{label}'")))?;

    number_text
        .parse::<u64>()
        .map_err(|e| SensorError::ReadFailed(format!("invalid number after '{label}': {e}")))
}

#[derive(Debug, Clone, Copy)]
pub struct MacMemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

pub trait MacMemorySource: Send {
    fn read(&mut self) -> Result<MacMemoryInfo, SensorError>;
}

pub fn available_bytes(vm_stat: &VmStat) -> u64 {
    (vm_stat.pages_free + vm_stat.pages_inactive + vm_stat.pages_speculative)
        * vm_stat.page_size_bytes
}

pub fn estimate_watts(mem: &MacMemoryInfo, watts_per_gb: f64) -> f64 {
    let used_bytes = mem.total_bytes.saturating_sub(mem.available_bytes);
    let used_gb = used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    used_gb * watts_per_gb
}

pub struct MacRamSensor<S: MacMemorySource> {
    source: S,
    watts_per_gb: f64,
}

impl<S: MacMemorySource> MacRamSensor<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            watts_per_gb: DEFAULT_WATTS_PER_GB,
        }
    }
}

impl<S: MacMemorySource> PowerSensor for MacRamSensor<S> {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let mem = self.source.read()?;

        Ok(SensorReading {
            component: Component::Ram,
            watts: estimate_watts(&mem, self.watts_per_gb),
            confidence: Confidence::Estimated,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::{available_bytes, parse_vm_stat, MacMemoryInfo, MacMemorySource};
    use crate::model::SensorError;
    use std::process::Command;

    pub struct RealMacMemorySource;

    fn run(cmd: &str, args: &[&str]) -> Result<String, SensorError> {
        let output = Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| SensorError::ReadFailed(format!("failed to run {cmd}: {e}")))?;

        String::from_utf8(output.stdout)
            .map_err(|e| SensorError::ReadFailed(format!("{cmd} output wasn't valid UTF-8: {e}")))
    }

    impl MacMemorySource for RealMacMemorySource {
        fn read(&mut self) -> Result<MacMemoryInfo, SensorError> {
            let vm_stat_text = run("vm_stat", &[])?;
            let vm_stat = parse_vm_stat(&vm_stat_text)?;

            let memsize_text = run("sysctl", &["-n", "hw.memsize"])?;
            let total_bytes = memsize_text
                .trim()
                .parse::<u64>()
                .map_err(|e| SensorError::ReadFailed(format!("invalid hw.memsize value: {e}")))?;

            Ok(MacMemoryInfo {
                total_bytes,
                available_bytes: available_bytes(&vm_stat),
            })
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos_impl::RealMacMemorySource;

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_VM_STAT: &str = r#"Mach Virtual Memory Statistics: (page size of 4096 bytes)
Pages free:                     92020.
Pages active:                  147234.
Pages inactive:                131889.
Pages speculative:               48201.
Pages throttled:                    0.
Pages wired down:              103829.
"#;

    #[test]
    fn parses_page_size_and_the_fields_this_sensor_needs() {
        let vm_stat = parse_vm_stat(SAMPLE_VM_STAT).unwrap();

        assert_eq!(vm_stat.page_size_bytes, 4096);
        assert_eq!(vm_stat.pages_free, 92020);
        assert_eq!(vm_stat.pages_inactive, 131889);
        assert_eq!(vm_stat.pages_speculative, 48201);
    }

    #[test]
    fn fails_clearly_when_a_needed_field_is_missing() {
        let result = parse_vm_stat("Mach Virtual Memory Statistics: (page size of 4096 bytes)\n");

        assert!(matches!(result.unwrap_err(), SensorError::ReadFailed(_)));
    }

    #[test]
    fn available_bytes_sums_free_inactive_and_speculative_pages() {
        let vm_stat = parse_vm_stat(SAMPLE_VM_STAT).unwrap();

        let available = available_bytes(&vm_stat);

        // (92020 + 131889 + 48201) pages * 4096 bytes/page
        assert_eq!(available, 272_110 * 4096);
    }

    #[test]
    fn estimates_more_watts_when_more_memory_is_used() {
        let mostly_free = MacMemoryInfo {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 15 * 1024 * 1024 * 1024,
        };
        let mostly_used = MacMemoryInfo {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 2 * 1024 * 1024 * 1024,
        };

        let watts_free = estimate_watts(&mostly_free, DEFAULT_WATTS_PER_GB);
        let watts_used = estimate_watts(&mostly_used, DEFAULT_WATTS_PER_GB);

        assert!(watts_used > watts_free);
    }

    struct FakeMacMemorySource {
        info: MacMemoryInfo,
    }

    impl MacMemorySource for FakeMacMemorySource {
        fn read(&mut self) -> Result<MacMemoryInfo, SensorError> {
            Ok(self.info)
        }
    }

    #[test]
    fn sample_reports_an_estimated_ram_reading() {
        let source = FakeMacMemorySource {
            info: MacMemoryInfo {
                total_bytes: 16 * 1024 * 1024 * 1024,
                available_bytes: 8 * 1024 * 1024 * 1024,
            },
        };
        let mut sensor = MacRamSensor::new(source);

        let reading = sensor.sample().unwrap();

        assert_eq!(reading.component, Component::Ram);
        assert_eq!(reading.confidence, Confidence::Estimated);
        assert!(reading.watts > 0.0);
    }
}
