use crate::model::{Component, Confidence, GpuVendor, PowerSensor, SensorError, SensorReading};
use nvml_wrapper::Nvml;

pub fn milliwatts_to_watts(mw: u32) -> f64 {
    mw as f64 / 1000.0
}

pub struct NvidiaSensor {
    nvml: Nvml,
    device_index: u32,
}

impl NvidiaSensor {
    pub fn init(device_index: u32) -> Result<Self, SensorError> {
        let nvml = Nvml::init()
            .map_err(|e| SensorError::Unavailable(format!("NVML not available: {e}")))?;

        // fail fast if the device doesn't exist, rather than on first sample
        nvml.device_by_index(device_index).map_err(|e| {
            SensorError::Unavailable(format!("no NVIDIA GPU at index {device_index}: {e}"))
        })?;

        Ok(Self { nvml, device_index })
    }
}

impl PowerSensor for NvidiaSensor {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let device = self
            .nvml
            .device_by_index(self.device_index)
            .map_err(|e| SensorError::ReadFailed(e.to_string()))?;

        let power_mw = device
            .power_usage()
            .map_err(|e| SensorError::ReadFailed(e.to_string()))?;

        Ok(SensorReading {
            component: Component::Gpu(GpuVendor::Nvidia),
            watts: milliwatts_to_watts(power_mw),
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_milliwatts_to_watts() {
        assert_eq!(milliwatts_to_watts(80_000), 80.0);
    }

    #[test]
    fn init_never_panics_regardless_of_whether_a_driver_is_present() {
        let _ = NvidiaSensor::init(0);
    }
}
