use crate::sampler::Sampler;

#[cfg(target_os = "linux")]
pub fn build_default_sampler() -> Sampler {
    use crate::discovery;
    use crate::sensors::gpu_power::LinuxGpuPowerSensor;
    use crate::sensors::nvidia::NvidiaSensor;
    use crate::sensors::ram::RamSensor;
    use crate::sensors::rapl::RaplSensor;
    use std::path::Path;

    let mut sampler = Sampler::new();

    let configured_powercap = std::env::var("POWERWATCH_POWERCAP_PATH").ok();
    let mut rapl_target = configured_powercap
        .as_deref()
        .and_then(|path| discovery::discover_rapl(Path::new(path)));

    if rapl_target.is_none() {
        for path in [
            "/sys/devices/virtual/powercap/intel-rapl",
            "/sys/devices/virtual/powercap",
            "/sys/class/powercap",
        ] {
            if let Some(target) = discovery::discover_rapl(Path::new(path)) {
                rapl_target = Some(target);
                break;
            }
        }
    }

    if let Some(target) = rapl_target {
        sampler.add_sensor(
            "cpu",
            Box::new(RaplSensor::new(target.energy_path, target.max_energy_uj)),
        );
    }

    // NVIDIA: enumerate every NVML device exposed by the host/runtime.
    if let Ok(devices) = NvidiaSensor::discover() {
        for (index, _) in devices {
            if let Ok(sensor) = NvidiaSensor::init(index) {
                sampler.add_sensor(format!("gpu:nvidia:{index}"), Box::new(sensor));
            }
        }
    }

    // AMD and Intel: enumerate every DRM hwmon device that exposes real power
    // or energy telemetry. The readings themselves come directly from hwmon
    // under /sys; no /dev/dri access is required for power telemetry.
    for target in discovery::discover_linux_gpu_hwmon(Path::new("/sys/class/hwmon")) {
        let sensor_name = format!("gpu:{}:{}", target.vendor.as_str(), target.index);
        sampler.add_sensor(sensor_name, Box::new(LinuxGpuPowerSensor::new(target)));
    }

    sampler.add_sensor(
        "ram",
        Box::new(RamSensor::new(Path::new("/proc/meminfo").to_path_buf())),
    );

    for (device, disk_type) in discovery::discover_disks(Path::new("/sys/block")) {
        let sensor_name = format!("disk:{device}");
        sampler.add_sensor(sensor_name, disk_sensor_for(device, disk_type));
    }

    sampler
}

#[cfg(target_os = "linux")]
fn disk_sensor_for(
    device: String,
    disk_type: crate::sensors::disk::DiskType,
) -> Box<dyn crate::model::PowerSensor> {
    use crate::discovery::nvme_controller_for_device;
    use crate::sensors::disk::DiskSensor;
    use crate::sensors::nvme_power_states::{
        fetch_power_state_table, NvmePowerStateDiskSensor, RealCurrentPowerStateSource,
    };
    use std::path::Path;

    if disk_type == crate::sensors::disk::DiskType::Nvme {
        if let Some(controller) = nvme_controller_for_device(&device) {
            let device_path = format!("/dev/{controller}");
            if let Ok(power_states) = fetch_power_state_table(&device_path) {
                let power_state_path = Path::new("/sys/class/nvme")
                    .join(&controller)
                    .join("power_state");
                return Box::new(NvmePowerStateDiskSensor::new(
                    power_states,
                    RealCurrentPowerStateSource::new(power_state_path),
                    device,
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

    if let Ok(devices) = NvidiaSensor::discover() {
        for (index, _) in devices {
            if let Ok(sensor) = NvidiaSensor::init(index) {
                sampler.add_sensor(format!("gpu:nvidia:{index}"), Box::new(sensor));
            }
        }
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
