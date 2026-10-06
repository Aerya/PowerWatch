use crate::sampler::Sampler;

#[cfg(target_os = "linux")]
pub fn build_default_sampler() -> Sampler {
    use crate::discovery;
    use crate::model::{Component, GpuVendor};
    use crate::sensors::gpu_power::LinuxGpuPowerSensor;
    use crate::sensors::nvidia::NvidiaSensor;
    use crate::sensors::ram::RamSensor;
    use crate::sensors::rapl::{RaplPackageMinusDomainSensor, RaplSensor};
    use crate::sensors::rapl_msr::RaplMsrSensor;
    use std::path::{Path, PathBuf};

    let mut sampler = Sampler::new();

    let configured_powercap = std::env::var("POWERWATCH_POWERCAP_PATH").ok();
    let mut rapl_domains = configured_powercap
        .as_deref()
        .and_then(|path| discovery::discover_rapl_domains(Path::new(path)));

    if rapl_domains.is_none() {
        for path in [
            "/sys/devices/virtual/powercap/intel-rapl",
            "/sys/devices/virtual/powercap",
            "/sys/class/powercap",
        ] {
            if let Some(target) = discovery::discover_rapl_domains(Path::new(path)) {
                rapl_domains = Some(target);
                break;
            }
        }
    }

    let linux_gpu_targets = discovery::discover_linux_gpu_hwmon(Path::new("/sys/class/hwmon"));
    let has_intel_hwmon = linux_gpu_targets
        .iter()
        .any(|target| target.vendor == GpuVendor::Intel);
    let intel_drm_driver_loaded =
        Path::new("/sys/module/i915").exists() || Path::new("/sys/module/xe").exists();
    let use_uncore_gpu_accounting = rapl_domains
        .as_ref()
        .is_some_and(|domains| domains.uncore.is_some())
        && (has_intel_hwmon || intel_drm_driver_loaded);

    if let Some(domains) = &rapl_domains {
        if use_uncore_gpu_accounting {
            let uncore = domains.uncore.as_ref().expect("uncore checked above");
            // package includes the integrated GPU domain. Subtract uncore from
            // the CPU reading whenever an Intel GPU reading will also be shown,
            // so the iGPU is not counted once in CPU and again as a GPU.
            sampler.add_sensor(
                "cpu",
                Box::new(RaplPackageMinusDomainSensor::new(
                    domains.package.energy_path.clone(),
                    domains.package.max_energy_uj,
                    uncore.energy_path.clone(),
                    uncore.max_energy_uj,
                )),
            );
        } else {
            sampler.add_sensor(
                "cpu",
                Box::new(RaplSensor::new(
                    domains.package.energy_path.clone(),
                    domains.package.max_energy_uj,
                )),
            );
        }
    } else {
        // Some kernels, notably Synology DSM on Gemini Lake, expose the raw
        // x86 MSR device without the Linux powercap/RAPL sysfs interface.
        // Read package energy directly from MSR 0x606/0x611 as a measured
        // fallback. CPU 0 is sufficient because the counter is package-wide.
        let msr_path = std::env::var("POWERWATCH_MSR_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/dev/cpu/0/msr"));
        if let Ok(sensor) = RaplMsrSensor::new(&msr_path) {
            sampler.add_sensor("cpu", Box::new(sensor));
        }
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
    for target in linux_gpu_targets {
        let sensor_name = format!("gpu:{}:{}", target.vendor.as_str(), target.index);
        sampler.add_sensor(sensor_name, Box::new(LinuxGpuPowerSensor::new(target)));
    }

    // Some Intel platforms (including Jasper Lake) expose no i915/xe hwmon
    // power counter but do expose a RAPL "uncore" child domain. Use it as a
    // measured iGPU fallback only when no Intel hwmon GPU was discovered.
    if !has_intel_hwmon && intel_drm_driver_loaded {
        if let Some(uncore) = rapl_domains.as_ref().and_then(|domains| domains.uncore.as_ref()) {
            sampler.add_sensor(
                "gpu:intel:0",
                Box::new(RaplSensor::new_for_component(
                    uncore.energy_path.clone(),
                    uncore.max_energy_uj,
                    Component::GpuDevice {
                        vendor: GpuVendor::Intel,
                        index: 0,
                        name: "Intel integrated GPU (RAPL uncore)".to_string(),
                    },
                )),
            );
        }
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
