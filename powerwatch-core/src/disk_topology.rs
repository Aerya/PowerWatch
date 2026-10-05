use crate::model::Component;

pub fn component_display_label(component: &Component) -> String {
    #[cfg(target_os = "linux")]
    if let Component::Disk(device) = component {
        if let Some(detail) = linux::disk_detail(device) {
            return format!("disk ({device}) — {detail}");
        }
    }

    component.label()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    #[derive(Debug, Clone)]
    struct MountEntry {
        major_minor: String,
        mount_point: String,
        fs_type: String,
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
        let mut referenced_partitions = BTreeSet::new();

        for mount in parse_mountinfo(mountinfo) {
            let Some(top_name) = block_name_for_major_minor(dev_block, &mount.major_minor) else {
                continue;
            };

            for (physical, layers) in paths_to_physical(class_block, &top_name) {
                if physical != device {
                    continue;
                }

                for layer in &layers {
                    if is_partition_of(device, layer) {
                        referenced_partitions.insert(layer.clone());
                    }
                }

                let mut parts = layers
                    .iter()
                    .map(|name| layer_label(class_block, name))
                    .collect::<Vec<_>>();
                parts.push(format!("{} [{}]", mount.mount_point, mount.fs_type));
                descriptions.insert(parts.join(" → "));
            }
        }

        for partition in partition_names(class_block, device) {
            if !referenced_partitions.contains(&partition) {
                descriptions.insert(format!("{partition} [unmounted]"));
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
                if left_fields.len() < 5 || right_fields.is_empty() {
                    return None;
                }

                Some(MountEntry {
                    major_minor: left_fields[2].to_string(),
                    mount_point: unescape_mountinfo(left_fields[4]),
                    fs_type: right_fields[0].to_string(),
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

    fn partition_names(class_block: &Path, device: &str) -> Vec<String> {
        let Ok(device_path) = fs::canonicalize(class_block.join(device)) else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(device_path) else {
            return Vec::new();
        };

        let mut partitions = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                path.join("partition")
                    .exists()
                    .then(|| entry.file_name().to_string_lossy().to_string())
            })
            .collect::<Vec<_>>();
        partitions.sort();
        partitions
    }

    fn is_partition_of(device: &str, candidate: &str) -> bool {
        if let Some(rest) = candidate.strip_prefix(device) {
            return !rest.is_empty()
                && (rest.chars().all(|c| c.is_ascii_digit())
                    || (rest.starts_with('p')
                        && rest[1..].chars().all(|c| c.is_ascii_digit())));
        }
        false
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
        }

        #[test]
        fn identifies_common_partition_names() {
            assert!(is_partition_of("sda", "sda1"));
            assert!(is_partition_of("nvme0n1", "nvme0n1p2"));
            assert!(is_partition_of("mmcblk0", "mmcblk0p1"));
            assert!(!is_partition_of("sda", "sdb1"));
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
