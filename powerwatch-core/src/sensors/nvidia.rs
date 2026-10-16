use crate::model::{Component, Confidence, GpuVendor, PowerSensor, SensorError, SensorReading};
use nvml_wrapper::enum_wrappers::device::Sampling;
use nvml_wrapper::enums::device::SampleValue;
use nvml_wrapper::struct_wrappers::device::Sample;
use nvml_wrapper::Nvml;

pub fn milliwatts_to_watts(mw: u32) -> f64 {
    mw as f64 / 1000.0
}

fn latest_power_sample_watts(samples: &[Sample]) -> Option<f64> {
    let sample = samples.iter().max_by_key(|sample| sample.timestamp)?;
    let milliwatts = match sample.value {
        SampleValue::U32(value) => value as f64,
        SampleValue::U64(value) => value as f64,
        SampleValue::I64(value) if value >= 0 => value as f64,
        SampleValue::F64(value) if value.is_finite() && value >= 0.0 => value,
        _ => return None,
    };
    Some(milliwatts / 1000.0)
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

        let watts = match device.power_usage() {
            Ok(power_mw) => milliwatts_to_watts(power_mw),
            Err(direct_error) => {
                let samples = device
                    .samples(Sampling::Power, None)
                    .map_err(|sample_error| {
                        SensorError::ReadFailed(format!(
                            "direct power read: {direct_error}; power samples: {sample_error}"
                        ))
                    })?;
                latest_power_sample_watts(&samples).ok_or_else(|| {
                    SensorError::ReadFailed(format!(
                        "direct power read: {direct_error}; no valid power samples"
                    ))
                })?
            }
        };

        Ok(SensorReading {
            component: Component::GpuDevice {
                vendor: GpuVendor::Nvidia,
                index: self.device_index,
                name: self.device_name.clone(),
            },
            watts,
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
    fn uses_newest_power_sample_in_milliwatts() {
        let samples = [
            Sample {
                timestamp: 20,
                value: SampleValue::U32(47_050),
            },
            Sample {
                timestamp: 10,
                value: SampleValue::U32(46_830),
            },
        ];
        assert_eq!(latest_power_sample_watts(&samples), Some(47.05));
        assert_eq!(latest_power_sample_watts(&[]), None);
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
