use crate::model::Component;

pub fn component_display_label(component: &Component) -> String {
    #[cfg(target_os = "linux")]
    if let Component::Disk(device) = component {
        let size = linux::disk_size_label(device);
        let detail = linux::disk_detail(device);
        return match (size, detail) {
            (Some(size), Some(detail)) => format!("disk ({device}) · {size} — {detail}"),
            (Some(size), None) => format!("disk ({device}) · {size}"),
            (None, Some(detail)) => format!("disk ({device}) — {detail}"),
            (None, None) => component.label(),
        };
    }

    component.label()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;

    #[derive(Debug, Clone)]
    struct MountEntry {
        major_minor: String,
        mount_point: String,
        fs_type: String,
        source: String,
    }

    pub fn disk_size_label(device: &str) -> Option<String> {
        disk_size_label_from(device, Path::new("/sys/class/block"))
    }

    fn disk_size_label_from(device: &str, class_block: &Path) -> Option<String> {
        let sectors = fs::read_to_string(class_block.join(device).join("size"))
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()?;
        let bytes = sectors.checked_mul(512)?;
        Some(format_capacity(bytes))
    }

    fn format_capacity(bytes: u64) -> String {
        const UNITS: &[(&str, u64)] = &[
            ("PiB", 1u64 << 50),
            ("TiB", 1u64 << 40),
            ("GiB", 1u64 << 30),
            ("MiB", 1u64 << 20),
            ("KiB", 1u64 << 10),
        ];

        for (unit, size) in UNITS {
            if bytes >= *size {
                return format!("{:.2} {unit}", bytes as f64 / *size as f64);
            }
        }
        format!("{bytes} B")
    }

    pub fn disk_detail(device: &str) -> Option<String> {
        let mountinfo = fs::read_to_string("/proc/1/mountinfo")
            .or_else(|_| fs::read_to_string("/proc/self/mountinfo"))
            .ok()?;
        disk_detail_from(
            device,
            Path::new("/sys/class/block"),
            Path::new("/sys/dev/block"),
            &mountinfo,
        )
    }

    fn disk_detail_from(
        device: &str,
        class_block: &Path,
        dev_block: &Path,
        mountinfo: &str,
    ) -> Option<String> {
        let mut descriptions = BTreeSet::new();
        let mut groups: BTreeMap<(Vec<String>, String), BTreeSet<String>> = BTreeMap::new();

        for mount in parse_mountinfo(mountinfo) {
            let technical = is_technical_mount(&mount.mount_point, &mount.fs_type);

            for top_name in candidate_block_names(class_block, dev_block, &mount) {
                let mut matched_candidate = false;

                for (physical, layers) in paths_to_physical(class_block, &top_name) {
                    if physical != device {
                        continue;
                    }

                    matched_candidate = true;

                    if !technical {
                        groups
                            .entry((layers, mount.fs_type.clone()))
                            .or_default()
                            .insert(mount.mount_point.clone());
                    }
                }

                if matched_candidate {
                    break;
                }
            }
        }

        for ((layers, fs_type), mount_points) in groups {
            let mut parts = layers
                .iter()
                .map(|name| layer_label(class_block, name))
                .collect::<Vec<_>>();

            let mut mounts = mount_points.into_iter().collect::<Vec<_>>();
            mounts.sort_by(|a, b| {
                mount_rank(a)
                    .cmp(&mount_rank(b))
                    .then_with(|| a.len().cmp(&b.len()))
                    .then_with(|| a.cmp(b))
            });

            if let Some(primary) = mounts.first() {
                let mut mount_label = format!("{primary} [{fs_type}]");
                if mounts.len() > 1 {
                    mount_label.push_str(&format!(" (+{})", mounts.len() - 1));
                }
                parts.push(mount_label);
            }

            if !parts.is_empty() {
                descriptions.insert(parts.join(" → "));
            }
        }

        if descriptions.is_empty() {
            None
        } else {
            Some(descriptions.into_iter().collect::<Vec<_>>().join(" ; "))
        }
    }

    fn parse_mountinfo(contents: &str) -> Vec<MountEntry> {
        contents
            .lines()
            .filter_map(|line| {
                let (left, right) = line.split_once(" - ")?;
                let left_fields = left.split_whitespace().collect::<Vec<_>>();
                let right_fields = right.split_whitespace().collect::<Vec<_>>();
                if left_fields.len() < 5 || right_fields.len() < 2 {
                    return None;
                }

                Some(MountEntry {
                    major_minor: left_fields[2].to_string(),
                    mount_point: unescape_mountinfo(left_fields[4]),
                    fs_type: right_fields[0].to_string(),
                    source: unescape_mountinfo(right_fields[1]),
                })
            })
            .collect()
    }

    fn unescape_mountinfo(value: &str) -> String {
        value
            .replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\012", "\n")
            .replace("\\134", "\\")
    }

    fn is_technical_mount(mount_point: &str, fs_type: &str) -> bool {
        const TECHNICAL_PREFIXES: &[&str] = &[
            "/var/lib/docker",
            "/var/lib/containers",
            "/var/lib/kubelet",
            "/var/lib/snapd/snap",
            "/run/docker",
            "/run/containers",
            "/run/user",
            "/run/credentials",
            "/snap",
        ];

        if matches!(fs_type, "overlay" | "fuse-overlayfs" | "squashfs") {
            return true;
        }

        TECHNICAL_PREFIXES.iter().any(|prefix| {
            mount_point == *prefix
                || mount_point
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    fn mount_rank(path: &str) -> u8 {
        if path == "/" {
            0
        } else if path.starts_with("/mnt/")
            || path.starts_with("/media/")
            || path.starts_with("/run/media/")
        {
            1
        } else if path == "/home" || path.starts_with("/home/") {
            2
        } else if path == "/srv" || path.starts_with("/srv/") {
            3
        } else if path.starts_with("/boot") {
            4
        } else {
            5
        }
    }

    fn candidate_block_names(
        class_block: &Path,
        dev_block: &Path,
        mount: &MountEntry,
    ) -> Vec<String> {
        let mut names = Vec::new();

        if let Some(name) = block_name_for_major_minor(dev_block, &mount.major_minor) {
            names.push(name);
        }

        if let Some(name) = block_name_for_source(class_block, &mount.source) {
            if !names.contains(&name) {
                names.push(name);
            }
        }

        names
    }

    fn block_name_for_source(class_block: &Path, source: &str) -> Option<String> {
        let relative = source.strip_prefix("/dev/")?;
        let source_name = Path::new(relative)
            .file_name()?
            .to_string_lossy()
            .to_string();

        if class_block.join(&source_name).exists() {
            return Some(source_name);
        }

        let entries = fs::read_dir(class_block).ok()?;
        for entry in entries.flatten() {
            let block_name = entry.file_name().to_string_lossy().to_string();
            if !block_name.starts_with("dm-") {
                continue;
            }

            let Ok(dm_name) = fs::read_to_string(entry.path().join("dm/name")) else {
                continue;
            };
            if dm_name.trim() == source_name {
                return Some(block_name);
            }
        }

        None
    }

    fn block_name_for_major_minor(dev_block: &Path, major_minor: &str) -> Option<String> {
        let path = fs::canonicalize(dev_block.join(major_minor)).ok()?;
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
    }

    fn paths_to_physical(class_block: &Path, name: &str) -> Vec<(String, Vec<String>)> {
        let slaves = slave_names(class_block, name);
        if !slaves.is_empty() {
            let mut paths = Vec::new();
            for slave in slaves {
                for (physical, mut layers) in paths_to_physical(class_block, &slave) {
                    layers.push(name.to_string());
                    paths.push((physical, layers));
                }
            }
            return paths;
        }

        if let Some(parent) = partition_parent(class_block, name) {
            let mut paths = paths_to_physical(class_block, &parent);
            for (_, layers) in &mut paths {
                layers.push(name.to_string());
            }
            return paths;
        }

        vec![(name.to_string(), Vec::new())]
    }

    fn slave_names(class_block: &Path, name: &str) -> Vec<String> {
        let Ok(entries) = fs::read_dir(class_block.join(name).join("slaves")) else {
            return Vec::new();
        };
        let mut names = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn partition_parent(class_block: &Path, name: &str) -> Option<String> {
        let block = class_block.join(name);
        if !block.join("partition").exists() {
            return None;
        }
        let canonical = fs::canonicalize(block).ok()?;
        canonical
            .parent()?
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
    }

    fn layer_label(class_block: &Path, name: &str) -> String {
        let block = class_block.join(name);

        if let Ok(level) = fs::read_to_string(block.join("md/level")) {
            return format!("{name} [{}]", level.trim().to_ascii_uppercase());
        }

        if name.starts_with("dm-") {
            let dm_name = fs::read_to_string(block.join("dm/name"))
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| name.to_string());
            let uuid = fs::read_to_string(block.join("dm/uuid"))
                .unwrap_or_default()
                .to_ascii_uppercase();
            if uuid.starts_with("LVM-") {
                return format!("{} [LVM]", decode_lvm_dm_name(&dm_name));
            }
            let kind = if uuid.starts_with("CRYPT-") {
                "dm-crypt"
            } else {
                "DM"
            };
            return format!("{dm_name} [{kind}]");
        }

        name.to_string()
    }


    fn decode_lvm_dm_name(name: &str) -> String {
        let chars = name.chars().collect::<Vec<_>>();
        let mut left = String::new();
        let mut right = String::new();
        let mut in_right = false;
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '-' {
                if i + 1 < chars.len() && chars[i + 1] == '-' {
                    if in_right {
                        right.push('-');
                    } else {
                        left.push('-');
                    }
                    i += 2;
                    continue;
                }
                if !in_right {
                    in_right = true;
                    i += 1;
                    continue;
                }
            }

            if in_right {
                right.push(chars[i]);
            } else {
                left.push(chars[i]);
            }
            i += 1;
        }

        if left.is_empty() || right.is_empty() {
            name.to_string()
        } else {
            format!("{left}/{right}")
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_mountinfo_and_decodes_mount_points() {
            let mounts = parse_mountinfo(
                "42 31 8:1 / /mnt/My\\040Disk rw,relatime - ext4 /dev/sda1 rw\n",
            );
            assert_eq!(mounts.len(), 1);
            assert_eq!(mounts[0].major_minor, "8:1");
            assert_eq!(mounts[0].mount_point, "/mnt/My Disk");
            assert_eq!(mounts[0].fs_type, "ext4");
            assert_eq!(mounts[0].source, "/dev/sda1");
        }

        #[test]
        fn filters_container_and_runtime_mounts_from_labels() {
            assert!(is_technical_mount(
                "/var/lib/docker/volumes/example/_data",
                "ext4"
            ));
            assert!(is_technical_mount(
                "/run/user/1000/psd/browser",
                "fuse-overlayfs"
            ));
            assert!(!is_technical_mount("/mnt/torrent", "ext4"));
            assert!(!is_technical_mount("/run/media/aerya/GAMES", "ntfs3"));
        }

        #[test]
        fn prefers_root_and_user_mounts_for_compact_labels() {
            assert!(mount_rank("/") < mount_rank("/home"));
            assert!(mount_rank("/mnt/data") < mount_rank("/var/log"));
        }

        #[cfg(unix)]
        fn make_fake_disk_tree(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
            use std::os::unix::fs::symlink;

            let physical = root.join("devices/sda");
            let class_block = root.join("class/block");
            let dev_block = root.join("dev/block");
            std::fs::create_dir_all(&physical).unwrap();
            std::fs::create_dir_all(&class_block).unwrap();
            std::fs::create_dir_all(&dev_block).unwrap();
            symlink(&physical, class_block.join("sda")).unwrap();

            for (partition, major_minor) in [("sda1", "8:1"), ("sda2", "8:2")] {
                let partition_path = physical.join(partition);
                std::fs::create_dir_all(&partition_path).unwrap();
                std::fs::write(partition_path.join("partition"), "1\n").unwrap();
                symlink(&partition_path, class_block.join(partition)).unwrap();
                symlink(&partition_path, dev_block.join(major_minor)).unwrap();
            }

            (class_block, dev_block)
        }

        #[cfg(unix)]
        #[test]
        fn hides_unmounted_partitions_when_one_partition_is_mounted() {
            let temp = tempfile::tempdir().unwrap();
            let (class_block, dev_block) = make_fake_disk_tree(temp.path());
            let mountinfo =
                "42 31 8:1 / /mnt/data rw,relatime - ext4 /dev/sda1 rw\n";

            let detail = disk_detail_from("sda", &class_block, &dev_block, mountinfo);

            assert_eq!(detail.as_deref(), Some("sda1 → /mnt/data [ext4]"));
            let detail = detail.unwrap();
            assert!(!detail.contains("sda2"));
            assert!(!detail.contains("unmounted"));
        }

        #[cfg(unix)]
        #[test]
        fn shows_no_partition_detail_when_nothing_is_mounted() {
            let temp = tempfile::tempdir().unwrap();
            let (class_block, dev_block) = make_fake_disk_tree(temp.path());

            let detail = disk_detail_from("sda", &class_block, &dev_block, "");

            assert_eq!(detail, None);
        }

        #[test]
        fn formats_disk_capacity_from_sysfs_sector_count() {
            let temp = tempfile::tempdir().unwrap();
            let disk = temp.path().join("sda");
            std::fs::create_dir_all(&disk).unwrap();
            std::fs::write(disk.join("size"), "7814037168\n").unwrap();

            assert_eq!(
                disk_size_label_from("sda", temp.path()).as_deref(),
                Some("3.64 TiB")
            );
        }

        #[test]
        fn decodes_lvm_device_mapper_names() {
            assert_eq!(decode_lvm_dm_name("vg0-root"), "vg0/root");
            assert_eq!(
                decode_lvm_dm_name("vg--data-lv--media"),
                "vg-data/lv-media"
            );
        }
    }
}
