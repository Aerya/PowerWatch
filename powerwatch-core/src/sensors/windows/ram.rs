use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};
use crate::sensors::ram::DEFAULT_WATTS_PER_GB;

#[derive(Debug, Clone, Copy)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

pub trait MemoryInfoSource: Send {
    fn read(&mut self) -> Result<MemoryInfo, SensorError>;
}

pub fn estimate_watts(mem: &MemoryInfo, watts_per_gb: f64) -> f64 {
    let used_bytes = mem.total_bytes.saturating_sub(mem.available_bytes);
    let used_gb = used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    used_gb * watts_per_gb
}

pub struct WindowsRamSensor<S: MemoryInfoSource> {
    source: S,
    watts_per_gb: f64,
}

impl<S: MemoryInfoSource> WindowsRamSensor<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            watts_per_gb: DEFAULT_WATTS_PER_GB,
        }
    }
}

impl<S: MemoryInfoSource> PowerSensor for WindowsRamSensor<S> {
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

#[cfg(target_os = "windows")]
mod win32 {
    use super::{MemoryInfo, MemoryInfoSource};
    use crate::model::SensorError;
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    pub struct WindowsMemoryInfoSource;

    impl MemoryInfoSource for WindowsMemoryInfoSource {
        fn read(&mut self) -> Result<MemoryInfo, SensorError> {
            let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
            status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;

            let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
            if ok == 0 {
                return Err(SensorError::ReadFailed(
                    "GlobalMemoryStatusEx failed".to_string(),
                ));
            }

            Ok(MemoryInfo {
                total_bytes: status.ullTotalPhys,
                available_bytes: status.ullAvailPhys,
            })
        }
    }
}

#[cfg(target_os = "windows")]
pub use win32::WindowsMemoryInfoSource;

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeMemoryInfoSource {
        info: MemoryInfo,
    }

    impl MemoryInfoSource for FakeMemoryInfoSource {
        fn read(&mut self) -> Result<MemoryInfo, SensorError> {
            Ok(self.info)
        }
    }

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn estimates_more_watts_when_more_memory_is_used() {
        let mostly_free = MemoryInfo {
            total_bytes: 16 * GB,
            available_bytes: 15 * GB,
        };
        let mostly_used = MemoryInfo {
            total_bytes: 16 * GB,
            available_bytes: 2 * GB,
        };

        let watts_free = estimate_watts(&mostly_free, DEFAULT_WATTS_PER_GB);
        let watts_used = estimate_watts(&mostly_used, DEFAULT_WATTS_PER_GB);

        assert!(watts_used > watts_free);
    }

    #[test]
    fn matches_the_expected_watts_per_gb_calculation() {
        let mem = MemoryInfo {
            total_bytes: 16 * GB,
            available_bytes: 8 * GB, // 8GB used
        };

        let watts = estimate_watts(&mem, 0.3);

        assert!((watts - 2.4).abs() < 0.0001); // 8GB * 0.3W/GB
    }

    #[test]
    fn sample_reports_an_estimated_ram_reading() {
        let source = FakeMemoryInfoSource {
            info: MemoryInfo {
                total_bytes: 16 * GB,
                available_bytes: 8 * GB,
            },
        };
        let mut sensor = WindowsRamSensor::new(source);

        let reading = sensor.sample().unwrap();

        assert_eq!(reading.component, Component::Ram);
        assert_eq!(reading.confidence, Confidence::Estimated);
        assert!(reading.watts > 0.0);
    }
}
