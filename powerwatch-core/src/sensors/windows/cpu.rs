use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};

#[derive(Debug, Clone, Copy)]
pub struct CpuTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

pub trait CpuTimesSource: Send {
    fn read(&mut self) -> Result<CpuTimes, SensorError>;
}

pub fn compute_utilization(before: &CpuTimes, after: &CpuTimes) -> f64 {
    let total_delta =
        (after.kernel as i64 - before.kernel as i64) + (after.user as i64 - before.user as i64);
    let idle_delta = after.idle as i64 - before.idle as i64;

    if total_delta <= 0 {
        return 0.0;
    }

    let busy_delta = total_delta - idle_delta;
    (busy_delta as f64 / total_delta as f64).clamp(0.0, 1.0)
}

pub struct WindowsCpuSensor<S: CpuTimesSource> {
    source: S,
    assumed_max_watts: f64,
    last: Option<CpuTimes>,
}

impl<S: CpuTimesSource> WindowsCpuSensor<S> {
    pub fn new(source: S, assumed_max_watts: f64) -> Self {
        Self {
            source,
            assumed_max_watts,
            last: None,
        }
    }
}

impl<S: CpuTimesSource> PowerSensor for WindowsCpuSensor<S> {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let times = self.source.read()?;

        let Some(last) = self.last.replace(times) else {
            return Err(SensorError::NotReady(
                "need a second sample to compute utilization".to_string(),
            ));
        };

        let utilization = compute_utilization(&last, &times);

        Ok(SensorReading {
            component: Component::Cpu,
            watts: utilization * self.assumed_max_watts,
            confidence: Confidence::Estimated,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(target_os = "windows")]
mod win32 {
    use super::{CpuTimes, CpuTimesSource};
    use crate::model::SensorError;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;

    fn filetime_to_u64(ft: &FILETIME) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    }

    pub struct WindowsCpuTimesSource;

    impl CpuTimesSource for WindowsCpuTimesSource {
        fn read(&mut self) -> Result<CpuTimes, SensorError> {
            let mut idle: FILETIME = unsafe { std::mem::zeroed() };
            let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
            let mut user: FILETIME = unsafe { std::mem::zeroed() };

            let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
            if ok == 0 {
                return Err(SensorError::ReadFailed("GetSystemTimes failed".to_string()));
            }

            Ok(CpuTimes {
                idle: filetime_to_u64(&idle),
                kernel: filetime_to_u64(&kernel),
                user: filetime_to_u64(&user),
            })
        }
    }
}

#[cfg(target_os = "windows")]
pub use win32::WindowsCpuTimesSource;

#[cfg(test)]
mod tests {
    use super::*;

    fn times(idle: u64, kernel: u64, user: u64) -> CpuTimes {
        CpuTimes { idle, kernel, user }
    }

    #[test]
    fn zero_busy_time_is_zero_utilization() {
        // kernel+user grew by 100, and idle grew by the same 100
        // everything was idle
        let before = times(0, 0, 0);
        let after = times(100, 60, 40);

        assert_eq!(compute_utilization(&before, &after), 0.0);
    }

    #[test]
    fn no_idle_growth_is_full_utilization() {
        let before = times(50, 0, 0);
        let after = times(50, 60, 40);

        assert_eq!(compute_utilization(&before, &after), 1.0);
    }

    #[test]
    fn half_idle_growth_is_half_utilization() {
        let before = times(0, 0, 0);
        let after = times(50, 60, 40);

        assert_eq!(compute_utilization(&before, &after), 0.5);
    }

    #[test]
    fn no_time_elapsed_does_not_divide_by_zero() {
        let before = times(10, 20, 30);
        let after = times(10, 20, 30);

        assert_eq!(compute_utilization(&before, &after), 0.0);
    }

    struct FakeCpuTimesSource {
        readings: std::collections::VecDeque<CpuTimes>,
    }

    impl FakeCpuTimesSource {
        fn new(readings: Vec<CpuTimes>) -> Self {
            Self {
                readings: readings.into(),
            }
        }
    }

    impl CpuTimesSource for FakeCpuTimesSource {
        fn read(&mut self) -> Result<CpuTimes, SensorError> {
            self.readings
                .pop_front()
                .ok_or_else(|| SensorError::ReadFailed("no more fake readings".to_string()))
        }
    }

    #[test]
    fn first_sample_returns_not_ready() {
        let source = FakeCpuTimesSource::new(vec![times(0, 0, 0)]);
        let mut sensor = WindowsCpuSensor::new(source, 65.0);

        let result = sensor.sample();

        assert!(matches!(result.unwrap_err(), SensorError::NotReady(_)));
    }

    #[test]
    fn second_sample_estimates_watts_from_utilization() {
        let source = FakeCpuTimesSource::new(vec![times(0, 0, 0), times(0, 60, 40)]);
        let mut sensor = WindowsCpuSensor::new(source, 65.0);

        sensor.sample().unwrap_err(); // warms up
        let reading = sensor.sample().unwrap();

        assert_eq!(reading.component, Component::Cpu);
        assert_eq!(reading.confidence, Confidence::Estimated);
        assert_eq!(reading.watts, 65.0); // 100% utilization * 65W assumed max
    }

    #[test]
    fn partial_utilization_scales_watts_proportionally() {
        let source = FakeCpuTimesSource::new(vec![times(0, 0, 0), times(50, 60, 40)]);
        let mut sensor = WindowsCpuSensor::new(source, 65.0);

        sensor.sample().unwrap_err();
        let reading = sensor.sample().unwrap();

        assert_eq!(reading.watts, 32.5); // 50% utilization * 65W
    }
}
