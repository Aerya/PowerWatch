use crate::sensors::disk::DiskType;
use std::path::{Path, PathBuf};

pub struct RaplTarget {
    pub energy_path: PathBuf,
    pub max_energy_uj: u64,
}

pub fn discover_rapl(powercap_dir: &Path) -> Option<RaplTarget> {
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

        let Ok(max_energy_text) = std::fs::read_to_string(path.join("max_energy_range_uj")) else {
            continue;
        };
        let Ok(max_energy_uj) = max_energy_text.trim().parse() else {
            continue;
        };

        let energy_path = path.join("energy_uj");
        if !energy_path.exists() {
            continue;
        }

        return Some(RaplTarget {
            energy_path,
            max_energy_uj,
        });
    }

    None
}

pub fn discover_amd_gpu(hwmon_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(hwmon_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = std::fs::read_to_string(path.join("name")).ok()?;

        if name.trim() == "amdgpu" {
            let power_path = path.join("power1_average");
            if power_path.exists() {
                return Some(power_path);
            }
        }
    }

    None
}

fn is_virtual_or_aggregate_device(name: &str) -> bool {
    name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("sr")
        || name.starts_with("dm-")
        || name.starts_with("md")
        || name.starts_with("zram")
        || name.starts_with("zd")
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

        // Avoid loop/optical devices, aggregate/virtual layers and eMMC
        // boot/RPMB pseudo devices so storage is not double-counted.
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
    fn finds_the_hwmon_device_named_amdgpu() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("hwmon0/name"), "coretemp\n");
        write(base.join("hwmon1/name"), "amdgpu\n");
        write(base.join("hwmon1/power1_average"), "45000000\n");

        let power_path = discover_amd_gpu(base).unwrap();

        assert_eq!(power_path, base.join("hwmon1/power1_average"));
    }

    #[test]
    fn returns_none_when_no_amdgpu_hwmon_device_exists() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write(base.join("hwmon0/name"), "coretemp\n");

        assert!(discover_amd_gpu(base).is_none());
    }

    #[test]
    fn discovers_all_physical_disks_and_skips_virtual_layers() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("loop0/queue/rotational"), "0\n");
        write(base.join("dm-0/queue/rotational"), "0\n");
        write(base.join("md0/queue/rotational"), "0\n");
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
