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

        // a sub-domain name has a second ':', e.g. "intel-rapl:0:0"
        if name.matches(':').count() != 1 {
            continue;
        }

        let domain_name = std::fs::read_to_string(path.join("name")).ok()?;
        if !domain_name.trim().starts_with("package") {
            continue;
        }

        let max_energy_uj = std::fs::read_to_string(path.join("max_energy_range_uj"))
            .ok()?
            .trim()
            .parse()
            .ok()?;

        return Some(RaplTarget {
            energy_path: path.join("energy_uj"),
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

pub fn discover_primary_disk(block_dir: &Path) -> Option<(String, DiskType)> {
    let entries = std::fs::read_dir(block_dir).ok()?;

    let mut candidates: Vec<_> = entries.flatten().collect();
    candidates.sort_by_key(|e| e.file_name());

    for entry in candidates {
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("sr") {
            continue;
        }

        let disk_type = if name.starts_with("nvme") {
            DiskType::Nvme
        } else {
            let rotational = std::fs::read_to_string(entry.path().join("queue/rotational")).ok()?;
            if rotational.trim() == "1" {
                DiskType::Hdd7200Rpm
            } else {
                DiskType::SsdSata
            }
        };

        return Some((name, disk_type));
    }

    None
}

pub fn discover_nvme_controller(nvme_class_dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(nvme_class_dir).ok()?;

    let mut candidates: Vec<_> = entries.flatten().collect();
    candidates.sort_by_key(|e| e.file_name());

    for entry in candidates {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("nvme") && name[4..].chars().all(|c| c.is_ascii_digit()) {
            return Some(name);
        }
    }

    None
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

        let target = discover_rapl(base).unwrap();

        assert_eq!(target.energy_path, base.join("intel-rapl:0/energy_uj"));
        assert_eq!(target.max_energy_uj, 262_143_328_850);
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
    fn picks_the_first_real_disk_and_skips_virtual_devices() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("loop0/queue/rotational"), "0\n");
        write(base.join("sda/queue/rotational"), "1\n");

        let (name, disk_type) = discover_primary_disk(base).unwrap();

        assert_eq!(name, "sda");
        assert_eq!(disk_type, DiskType::Hdd7200Rpm);
    }

    #[test]
    fn treats_nvme_named_devices_as_nvme_without_checking_rotational() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        write(base.join("nvme0n1/queue/rotational"), "0\n");

        let (name, disk_type) = discover_primary_disk(base).unwrap();

        assert_eq!(name, "nvme0n1");
        assert_eq!(disk_type, DiskType::Nvme);
    }

    #[test]
    fn returns_none_when_only_virtual_devices_are_present() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write(base.join("loop0/queue/rotational"), "0\n");

        assert!(discover_primary_disk(base).is_none());
    }

    #[test]
    fn finds_the_nvme_controller_and_skips_namespaces() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        fs::create_dir_all(base.join("nvme0n1")).unwrap();
        fs::create_dir_all(base.join("nvme0")).unwrap();

        let controller = discover_nvme_controller(base).unwrap();

        assert_eq!(controller, "nvme0");
    }

    #[test]
    fn returns_none_when_theres_no_nvme_class_directory() {
        let dir = tempfile::tempdir().unwrap();

        assert!(discover_nvme_controller(&dir.path().join("does-not-exist")).is_none());
    }
}
