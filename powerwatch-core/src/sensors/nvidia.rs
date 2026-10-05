use crate::model::{Component, Confidence, GpuVendor, PowerSensor, SensorError, SensorReading};
use nvml_wrapper::Nvml;

pub fn milliwatts_to_watts(mw: u32) -> f64 {
    mw as f64 / 1000.0
}

pub struct NvidiaSensor {
    nvml: Nvml,
    device_index: u32,
    device_name: String,
}

impl NvidiaSensor {
    pub fn discover() -> Result<Vec<(u32, String)>, SensorError> {
        let nvml = Nvml::init()
            .map_err(|e| SensorError::Unavailable(format!("NVML not available: {e}")))?;
        let count = nvml
            .device_count()
            .map_err(|e| SensorError::Unavailable(format!("cannot enumerate NVIDIA GPUs: {e}")))?;

        let mut devices = Vec::new();
        for index in 0..count {
            let Ok(device) = nvml.device_by_index(index) else {
                continue;
            };
            let name = device
                .name()
                .unwrap_or_else(|_| format!("NVIDIA GPU {index}"));
            devices.push((index, name));
        }
        Ok(devices)
    }

    pub fn init(device_index: u32) -> Result<Self, SensorError> {
        let nvml = Nvml::init()
            .map_err(|e| SensorError::Unavailable(format!("NVML not available: {e}")))?;

        let device_name = {
            let device = nvml.device_by_index(device_index).map_err(|e| {
                SensorError::Unavailable(format!("no NVIDIA GPU at index {device_index}: {e}"))
            })?;
            device
                .name()
                .unwrap_or_else(|_| format!("NVIDIA GPU {device_index}"))
        };

        Ok(Self {
            nvml,
            device_index,
            device_name,
        })
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
            component: Component::GpuDevice {
                vendor: GpuVendor::Nvidia,
                index: self.device_index,
                name: self.device_name.clone(),
            },
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
    fn discovery_never_panics_regardless_of_whether_a_driver_is_present() {
        let _ = NvidiaSensor::discover();
    }

    #[test]
    fn init_never_panics_regardless_of_whether_a_driver_is_present() {
        let _ = NvidiaSensor::init(0);
    }
}
