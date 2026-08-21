use crate::sampler::Sampler;

#[cfg(target_os = "linux")]
pub fn build_default_sampler() -> Sampler {
    use crate::discovery;
    use crate::sensors::amd::AmdGpuSensor;
    use crate::sensors::nvidia::NvidiaSensor;
    use crate::sensors::ram::RamSensor;
    use crate::sensors::rapl::RaplSensor;
    use std::path::Path;

    let mut sampler = Sampler::new();

    if let Some(target) = discovery::discover_rapl(Path::new("/sys/class/powercap")) {
        sampler.add_sensor(
            "cpu",
            Box::new(RaplSensor::new(target.energy_path, target.max_energy_uj)),
        );
    }

    if let Ok(nvidia) = NvidiaSensor::init(0) {
        sampler.add_sensor("gpu", Box::new(nvidia));
    } else if let Some(power_path) = discovery::discover_amd_gpu(Path::new("/sys/class/hwmon")) {
        sampler.add_sensor("gpu", Box::new(AmdGpuSensor::new(power_path)));
    }

    sampler.add_sensor(
        "ram",
        Box::new(RamSensor::new(Path::new("/proc/meminfo").to_path_buf())),
    );

    if let Some((device, disk_type)) = discovery::discover_primary_disk(Path::new("/sys/block")) {
        sampler.add_sensor("disk", disk_sensor_for(device, disk_type));
    }

    sampler
}

#[cfg(target_os = "linux")]
fn disk_sensor_for(
    device: String,
    disk_type: crate::sensors::disk::DiskType,
) -> Box<dyn crate::model::PowerSensor> {
    use crate::discovery::discover_nvme_controller;
    use crate::sensors::disk::DiskSensor;
    use crate::sensors::nvme_power_states::{
        fetch_power_state_table, NvmePowerStateDiskSensor, RealCurrentPowerStateSource,
    };
    use std::path::Path;

    if disk_type == crate::sensors::disk::DiskType::Nvme {
        if let Some(controller) = discover_nvme_controller(Path::new("/sys/class/nvme")) {
            let device_path = format!("/dev/{controller}");
            if let Ok(power_states) = fetch_power_state_table(&device_path) {
                let power_state_path = Path::new("/sys/class/nvme")
                    .join(&controller)
                    .join("power_state");
                return Box::new(NvmePowerStateDiskSensor::new(
                    power_states,
                    RealCurrentPowerStateSource::new(power_state_path),
                    controller,
                ));
            }
        }
    }

    Box::new(DiskSensor::new(
        Path::new("/proc/diskstats").to_path_buf(),
        device,
        disk_type,
    ))
}

#[cfg(target_os = "windows")]
pub fn build_default_sampler() -> Sampler {
    use crate::sensors::disk::DiskType;
    use crate::sensors::nvidia::NvidiaSensor;
    use crate::sensors::windows::cpu::{WindowsCpuSensor, WindowsCpuTimesSource};
    use crate::sensors::windows::disk::{WindowsDiskActivitySource, WindowsDiskSensor};
    use crate::sensors::windows::lhm::{LibreHardwareMonitorCpuSensor, RealLhmPowerSource};
    use crate::sensors::windows::ram::{WindowsMemoryInfoSource, WindowsRamSensor};

    let mut sampler = Sampler::new();

    const ASSUMED_MAX_CPU_WATTS: f64 = 65.0;
    let cpu_sensor: Box<dyn crate::model::PowerSensor> = match RealLhmPowerSource::new() {
        Ok(source) => Box::new(LibreHardwareMonitorCpuSensor::new(source)),
        Err(_) => Box::new(WindowsCpuSensor::new(
            WindowsCpuTimesSource,
            ASSUMED_MAX_CPU_WATTS,
        )),
    };
    sampler.add_sensor("cpu", cpu_sensor);

    if let Ok(nvidia) = NvidiaSensor::init(0) {
        sampler.add_sensor("gpu", Box::new(nvidia));
    }

    sampler.add_sensor(
        "ram",
        Box::new(WindowsRamSensor::new(WindowsMemoryInfoSource)),
    );

    if let Ok(source) = WindowsDiskActivitySource::new() {
        sampler.add_sensor(
            "disk",
            Box::new(WindowsDiskSensor::new(source, DiskType::SsdSata)),
        );
    }

    sampler
}

#[cfg(target_os = "macos")]
pub fn build_default_sampler() -> Sampler {
    use crate::model::{Component, GpuVendor};
    use crate::sensors::disk::DiskType;
    use crate::sensors::macos::disk::{MacDiskSensor, RealDiskActivitySource};
    use crate::sensors::macos::powermetrics::{MacPowerMetricsSensor, RealPowerMetricsRunner};
    use crate::sensors::macos::ram::{MacRamSensor, RealMacMemorySource};

    let mut sampler = Sampler::new();

    sampler.add_sensor(
        "cpu",
        Box::new(MacPowerMetricsSensor::new(
            RealPowerMetricsRunner,
            "cpu_power",
            "cpu_power",
            Component::Cpu,
        )),
    );

    sampler.add_sensor(
        "gpu",
        Box::new(MacPowerMetricsSensor::new(
            RealPowerMetricsRunner,
            "gpu_power",
            "gpu_power",
            Component::Gpu(GpuVendor::Apple),
        )),
    );

    sampler.add_sensor("ram", Box::new(MacRamSensor::new(RealMacMemorySource)));

    sampler.add_sensor(
        "disk",
        Box::new(MacDiskSensor::new(
            RealDiskActivitySource,
            DiskType::SsdSata,
        )),
    );

    sampler
}
