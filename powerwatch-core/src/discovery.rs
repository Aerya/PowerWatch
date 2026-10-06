use crate::model::GpuVendor;
use crate::sensors::disk::DiskType;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaplTarget {
    pub energy_path: PathBuf,
    pub max_energy_uj: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaplDomains {
    pub package: RaplTarget,
    pub uncore: Option<RaplTarget>,
}

fn rapl_target_at(path: &Path) -> Option<RaplTarget> {
    let max_energy_text = std::fs::read_to_string(path.join("max_energy_range_uj")).ok()?;
    let max_energy_uj = max_energy_text.trim().parse().ok()?;
    let energy_path = path.join("energy_uj");
    energy_path.exists().then_some(RaplTarget {
        energy_path,
        max_energy_uj,
    })
}

pub fn discover_rapl(powercap_dir: &Path) -> Option<RaplTarget> {
    discover_rapl_domains(powercap_dir).map(|domains| domains.package)
}

pub fn discover_rapl_domains(powercap_dir: &Path) -> Option<RaplDomains> {
    let entries = std::fs::read_dir(powercap_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // A sub-domain has a second ':', e.g. "intel-rapl:0:0".
        if name.matches(':').count() != 1 {
            continue;
        }

        // Some container/sysfs layouts expose entries that are present but
        // unreadable or broken symlinks. Skip those entries instead of
        // aborting discovery for the whole RAPL tree.
        let Ok(domain_name) = std::fs::read_to_string(path.join("name")) else {
            continue;
        };
        if !domain_name.trim().starts_with("package") {
            continue;
        }

        let Some(package) = rapl_target_at(&path) else {
            continue;
        };

        let uncore = std::fs::read_dir(&path).ok().and_then(|children| {
            children.flatten().find_map(|child| {
                let child_path = child.path();
                let domain_name = std::fs::read_to_string(child_path.join("name")).ok()?;
                (domain_name.trim() == "uncore")
                    .then(|| rapl_target_at(&child_path))
                    .flatten()
            })
        });

        return Some(RaplDomains { package, uncore });
    }

    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuPowerSource {
    PowerUw(PathBuf),
    EnergyUj(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxGpuTarget {
    pub vendor: GpuVendor,
    pub index: u32,
    pub name: String,
    pub source: GpuPowerSource,
}

fn hwmon_gpu_name(path: &Path, driver_name: &str, index: u32) -> String {
    let bus_id = std::fs::canonicalize(path.join("device"))
        .ok()
        .and_then(|device| device.file_name().map(|value| value.to_string_lossy().to_string()))
        .filter(|value| value.contains(':') && value.contains('.'));

    match bus_id {
        Some(bus_id) => format!("{driver_name} {bus_id}"),
        None => format!("{driver_name} GPU {index}"),
    }
}

pub fn discover_linux_gpu_hwmon(hwmon_dir: &Path) -> Vec<LinuxGpuTarget> {
    let Ok(entries) = std::fs::read_dir(hwmon_dir) else {
        return Vec::new();
    };

    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| {
        std::fs::canonicalize(entry.path().join("device"))
            .ok()
            .and_then(|device| device.file_name().map(|value| value.to_os_string()))
            .unwrap_or_else(|| entry.file_name())
    });

    let mut amd_index = 0u32;
    let mut intel_index = 0u32;
    let mut targets = Vec::new();

    for entry in entries {
        let path = entry.path();
        let Ok(driver_name_raw) = std::fs::read_to_string(path.join("name")) else {
            continue;
        };
        let driver_name = driver_name_raw.trim();

        let (vendor, index) = match driver_name {
            "amdgpu" => {
                let index = amd_index;
                amd_index += 1;
                (GpuVendor::Amd, index)
            }
            // Device-level Intel DRM hwmon instances. i915_gtN is intentionally
            // excluded to avoid counting GT subdevices in addition to the card.
            "i915" | "xe" => {
                let index = intel_index;
                intel_index += 1;
                (GpuVendor::Intel, index)
            }
            _ => continue,
        };

        let source = ["power1_average", "power1_input"]
            .into_iter()
            .map(|name| path.join(name))
            .find(|candidate| candidate.exists())
            .map(GpuPowerSource::PowerUw)
            .or_else(|| {
                let candidate = path.join("energy1_input");
                candidate.exists().then_some(GpuPowerSource::EnergyUj(candidate))
            });

        let Some(source) = source else {
            continue;
        };

        let name = hwmon_gpu_name(&path, driver_name, index);
        targets.push(LinuxGpuTarget {
            vendor,
            index,
            name,
            source,
        });
    }

    targets
}

fn is_virtual_or_aggregate_device(name: &str) -> bool {
    name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("sr")
        || name.starts_with("dm-")
        || name.starts_with("md")
        || name.starts_with("zram")
        || name.starts_with("zd")
        || name.starts_with("synoboot")
        || (name.starts_with("mmcblk")
            && (name.contains("boot") || name.ends_with("rpmb")))
}

pub fn discover_disks(block_dir: &Path) -> Vec<(String, DiskType)> {
    let Ok(entries) = std::fs::read_dir(block_dir) else {
        return Vec::new();
    };

    let mut candidates: Vec<_> = entries.flatten().collect();
    candidates.sort_by_key(|e| e.file_name());

    let mut disks = Vec::new();

    for entry in candidates {
        let name = entry.file_name().to_string_lossy().to_string();

        // Avoid loop/optical devices, aggregate/virtual layers, Synology
        // synoboot pseudo devices and eMMC boot/RPMB devices so storage is
        // not double-counted.
        if is_virtual_or_aggregate_device(&name) {
            continue;
        }

        let disk_type = if name.starts_with("nvme") {
            DiskType::Nvme
        } else {
            let Ok(rotational) =
                std::fs::read_to_string(entry.path().join("queue/rotational"))
            else {
                continue;
            };

            if rotational.trim() == "1" {
                DiskType::Hdd7200Rpm
            } else {
                DiskType::SsdSata
            }
        };

        disks.push((name, disk_type));
    }

    disks
}

pub fn nvme_controller_for_device(device_name: &str) -> Option<String> {
    let rest = device_name.strip_prefix("nvme")?;
    let digit_count = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count == 0 {
        return None;
    }

    let (controller_index, suffix) = rest.split_at(digit_count);
    if !suffix.starts_with('n') {
        return None;
    }

    Some(format!("nvme{controller_index}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: PathBuf, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn finds_the_package_domain_and_skips_sub_domains() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("intel-rapl:0:0/name"), "core\n");
        write(base.join("intel-rapl:0/name"), "package-0\n");
        write(
            base.join("intel-rapl:0/max_energy_range_uj"),
            "262143328850\n",
        );
        write(base.join("intel-rapl:0/energy_uj"), "12345\n");

        let target = discover_rapl(base).unwrap();

        assert_eq!(target.energy_path, base.join("intel-rapl:0/energy_uj"));
        assert_eq!(target.max_energy_uj, 262_143_328_850);
    }

    #[test]
    fn discovers_uncore_as_an_optional_package_subdomain() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("intel-rapl:0/name"), "package-0\n");
        write(base.join("intel-rapl:0/max_energy_range_uj"), "1000000\n");
        write(base.join("intel-rapl:0/energy_uj"), "100\n");
        write(base.join("intel-rapl:0/intel-rapl:0:0/name"), "core\n");
        write(
            base.join("intel-rapl:0/intel-rapl:0:0/max_energy_range_uj"),
            "1000000\n",
        );
        write(base.join("intel-rapl:0/intel-rapl:0:0/energy_uj"), "80\n");
        write(base.join("intel-rapl:0/intel-rapl:0:1/name"), "uncore\n");
        write(
            base.join("intel-rapl:0/intel-rapl:0:1/max_energy_range_uj"),
            "1000000\n",
        );
        write(base.join("intel-rapl:0/intel-rapl:0:1/energy_uj"), "20\n");

        let domains = discover_rapl_domains(base).unwrap();
        assert_eq!(domains.package.energy_path, base.join("intel-rapl:0/energy_uj"));
        assert_eq!(
            domains.uncore.unwrap().energy_path,
            base.join("intel-rapl:0/intel-rapl:0:1/energy_uj")
        );
    }

    #[test]
    fn skips_broken_rapl_entries_and_keeps_searching() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("intel-rapl:0/name"), "package-broken\n");
        write(base.join("intel-rapl:1/name"), "package-1\n");
        write(base.join("intel-rapl:1/max_energy_range_uj"), "1000000\n");
        write(base.join("intel-rapl:1/energy_uj"), "42\n");

        let target = discover_rapl(base).unwrap();

        assert_eq!(target.energy_path, base.join("intel-rapl:1/energy_uj"));
    }

    #[test]
    fn returns_none_when_no_rapl_domain_exists() {
        let dir = tempfile::tempdir().unwrap();

        assert!(discover_rapl(dir.path()).is_none());
    }

    #[test]
    fn discovers_multiple_amd_and_intel_gpu_hwmon_devices() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("hwmon0/name"), "amdgpu\n");
        write(base.join("hwmon0/power1_average"), "45000000\n");
        write(base.join("hwmon1/name"), "amdgpu\n");
        write(base.join("hwmon1/power1_input"), "55000000\n");
        write(base.join("hwmon2/name"), "i915\n");
        write(base.join("hwmon2/energy1_input"), "123456\n");
        write(base.join("hwmon3/name"), "i915_gt0\n");
        write(base.join("hwmon3/energy1_input"), "123456\n");

        let gpus = discover_linux_gpu_hwmon(base);

        assert_eq!(gpus.len(), 3);
        assert_eq!(gpus[0].vendor, GpuVendor::Amd);
        assert_eq!(gpus[0].index, 0);
        assert_eq!(gpus[1].vendor, GpuVendor::Amd);
        assert_eq!(gpus[1].index, 1);
        assert_eq!(gpus[2].vendor, GpuVendor::Intel);
        assert_eq!(gpus[2].index, 0);
        assert!(matches!(gpus[2].source, GpuPowerSource::EnergyUj(_)));
    }

    #[test]
    fn ignores_gpu_hwmon_entries_without_power_or_energy_telemetry() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write(base.join("hwmon0/name"), "i915\n");

        assert!(discover_linux_gpu_hwmon(base).is_empty());
    }

    #[test]
    fn discovers_all_physical_disks_and_skips_virtual_layers() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("loop0/queue/rotational"), "0\n");
        write(base.join("dm-0/queue/rotational"), "0\n");
        write(base.join("md0/queue/rotational"), "0\n");
        write(base.join("synoboot/queue/rotational"), "1\n");
        write(base.join("synoboot1/queue/rotational"), "1\n");
        write(base.join("mmcblk0boot0/queue/rotational"), "0\n");
        write(base.join("mmcblk0boot1/queue/rotational"), "0\n");
        write(base.join("mmcblk0rpmb/queue/rotational"), "0\n");
        write(base.join("mmcblk0/queue/rotational"), "0\n");
        write(base.join("sda/queue/rotational"), "1\n");
        write(base.join("sdb/queue/rotational"), "0\n");
        write(base.join("nvme0n1/queue/rotational"), "0\n");
        write(base.join("nvme1n1/queue/rotational"), "0\n");

        let disks = discover_disks(base);

        assert_eq!(
            disks,
            vec![
                ("mmcblk0".to_string(), DiskType::SsdSata),
                ("nvme0n1".to_string(), DiskType::Nvme),
                ("nvme1n1".to_string(), DiskType::Nvme),
                ("sda".to_string(), DiskType::Hdd7200Rpm),
                ("sdb".to_string(), DiskType::SsdSata),
            ]
        );
    }

    #[test]
    fn returns_an_empty_list_when_no_disk_directory_exists() {
        let dir = tempfile::tempdir().unwrap();

        assert!(discover_disks(&dir.path().join("does-not-exist")).is_empty());
    }

    #[test]
    fn derives_nvme_controller_from_namespace_device() {
        assert_eq!(
            nvme_controller_for_device("nvme0n1"),
            Some("nvme0".to_string())
        );
        assert_eq!(
            nvme_controller_for_device("nvme12n3"),
            Some("nvme12".to_string())
        );
        assert_eq!(nvme_controller_for_device("sda"), None);
    }
}
