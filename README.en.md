# PowerWatch

<p align="center">
  <img src="docs/images/powerwatch-aerya-logo.png" alt="PowerWatch" width="720">
</p>

<p align="center">
  <a href="README.md">Français</a> · <strong>English</strong>
</p>

<p align="center">
  <img src="docs/images/PowerWatch-Hub-Overview.png" alt="PowerWatch Hub overview with three federated machines" width="1200">
</p>

<p align="center">
  <em>PowerWatch Hub centralizes the power usage of multiple machines in one dashboard.</em>
</p>

## Contents

- [Features](#features)
- [Measurements](#measurements)
  - [RAM and power supply](#ram-and-power-supply)
- [Docker installation](#docker-installation)
  - [Requirements](#requirements)
  - [Quick start](#quick-start)
  - [CPU RAPL via MSR](#cpu-rapl-via-msr--synology-dsm-and-kernels-without-powercap)
- [Native compilation](#native-compilation)
- [GPU](#gpu)
  - [AMD](#amd)
  - [Intel](#intel)
  - [NVIDIA](#nvidia)
- [Disks, partitions, and storage](#disks-partitions-and-storage)
- [Web UI](#web-ui)
- [History](#history)
- [Alerts and notifications](#alerts-and-notifications)
- [CLI and TUI in Docker](#cli-and-tui-in-docker)
- [Check detected sensors](#check-detected-sensors)
- [PowerWatch Hub](#powerwatch-hub--multiple-machines-one-dashboard)
  - [Start the Hub](#start-the-hub)
- [Security](#security)
- [Attribution](#attribution)
- [License](#license)

**PowerWatch** monitors the power consumption of a Linux machine from Docker and displays it in a Web UI, with history, alerts, and several measured or estimated hardware sources.

This fork is primarily designed for **servers, mini PCs, Linux desktop machines, and Docker hosts**. **Docker remains the recommended and best-controlled deployment method**; native compilation from source is also documented for advanced users.

> **Security:** the Web UI has no built-in authentication. Use it only on a **trusted private LAN**. Do not expose it directly to the Internet.

## Features

- Real-time Web UI with historical views.
- Persistent SQLite history with long-term aggregation.
- Alerts configurable from the Web UI.
- **Discord** and **Apprise API** notifications.
- Multi-GPU **NVIDIA / AMD / Intel** detection.
- **Intel RAPL `uncore` fallback** for some iGPUs without i915/xe hwmon counters.
- CPU **RAPL fallback through `/dev/cpu/0/msr`** when the kernel does not expose `powercap` (including some Synology DSM systems).
- Monitoring of all **physical disks** without double-counting RAID/LVM layers, including their capacity.
- Best-effort display of associated **mounted partitions, filesystems, mdraid, LVM, and dm-crypt**, without noise from unmounted partitions.
- CLI and TUI available in the Docker image.
- Multi-architecture **amd64 / arm64** GHCR images.
- **French / English** Web UI.
- Configurable PowerWatch instance name, automatically suggested when adding the instance to the Hub while keeping the Hub alias independent.
- CPU model with a direct **CPU Benchmark / PassMark** search link and monochrome CPU/RAM/disk icons.
- Best-effort RAM inventory: **installed capacity, populated memory-device count, and per-module details** through SMBIOS when available.
- Best-effort power-supply information through **SMBIOS Type 39**: nominal maximum capacity, manufacturer/model, location, type, and status when firmware reports them.
- **PowerWatch Hub**: aggregate multiple machines into one federated dashboard.

## Measurements

| Component | Source | Type |
|---|---|---|
| CPU | Linux RAPL (`powercap` or MSR `/dev/cpu/0/msr`) | Measured |
| NVIDIA GPU | NVML | Measured |
| AMD GPU | `amdgpu` hwmon | Measured |
| Intel GPU | `i915` / `xe` hwmon or RAPL `uncore` | Measured |
| RAM | Heuristic | Estimated |
| Disks | Busy time (`/proc/diskstats`) + media profile; NVMe power states when available | Estimated |
| Total | Sum of available sensors | Mixed |

Unavailable sensors are simply ignored.

On some Intel systems, PowerWatch can use the RAPL `uncore` subdomain as the iGPU power reading. In that case, CPU power is calculated from the package value minus `uncore` so the iGPU is not counted twice.

When the Linux `powercap` interface is absent but `/dev/cpu/0/msr` exists, PowerWatch can directly read the RAPL `MSR_RAPL_POWER_UNIT` (`0x606`) and `MSR_PKG_ENERGY_STATUS` (`0x611`) registers. This remains a **hardware-measured** CPU-package value, not an estimate based on CPU utilization.

### RAM and power supply

PowerWatch complements electrical readings with a small best-effort hardware inventory shown in the local Web UI and forwarded to the Hub.

For **RAM**:

- usable total memory is read from `/proc/meminfo`;
- PowerWatch first reads the kernel's raw SMBIOS/DMI table to recover physically installed capacity and populated memory devices;
- the interface **groups identical populated devices** into a compact summary (e.g. `2 × 16 GiB · DDR4 · 3200 MT/s`), without displaying long vendor part numbers or slot locations; full metadata remains accessible in the API;
- the displayed count is the number of populated **SMBIOS Memory Devices**. On some systems, soldered memory can therefore appear as a module even though it is not a removable DIMM;
- if SMBIOS is unavailable, PowerWatch displays **usable** Linux memory and no longer labels that fallback value as “Installed RAM”.

For the **power supply**:

- PowerWatch reads SMBIOS **Type 39 / System Power Supply**;
- generic firmware entries (`Default string`, `OEM Define`, status/type alone without credible identity or capacity) and explicitly absent PSU bays are hidden;
- the Web UI and Hub display **one compact PSU summary**, without repeating the wattage on the same line; optional details remain available in a tooltip;
- reported capacities are explicitly labeled **“SMBIOS unverified”**: even a plausible value may not match the real PSU rating;
- the watt value is an **SMBIOS-reported nominal maximum capacity, not physically verified**, not live electrical consumption;
- nominal PSU capacity is **never added** to the PowerWatch consumption total.

Docker masks `/sys/firmware` inside containers by default. The PowerWatch Compose therefore mounts the host firmware read-only at `/host-sys-firmware` and sets `POWERWATCH_DMI_TABLE_PATH=/host-sys-firmware/dmi/tables/DMI`. PowerWatch parses that raw table directly, so neither `privileged` mode nor `/dev/mem` access is required.

Many consumer motherboards and standard ATX PSUs do not expose SMBIOS Type 39 at all. PowerWatch now distinguishes **SMBIOS unavailable** from **PSU not exposed by SMBIOS firmware** instead of treating both cases as the same condition. `dmidecode` remains available as a fallback when the raw table cannot be read.

## Docker installation

### Requirements

- Linux;
- Docker Engine;
- Docker Compose;
- read access to `/sys` from the container;
- for NVIDIA: NVIDIA driver + NVIDIA Container Toolkit on the host.

### Quick start

```bash
git clone https://github.com/Aerya/PowerWatch.git
cd PowerWatch

cp .env.example .env
docker compose pull
docker compose up -d
```

By default, the Compose file exposes the Web UI only on:

```text
127.0.0.1:3000
```

To make it available on your local network, edit `.env` and set the **private IPv4 address of the server**:

```env
POWERWATCH_BIND_IP=192.168.0.50
```

Then restart:

```bash
docker compose up -d
```

The Web UI will then be available at:

```text
http://192.168.0.50:3000
```

The supplied [`compose.yaml`](compose.yaml) is the reference deployment file.

### CPU RAPL via MSR — Synology DSM and kernels without `powercap`

On some Intel systems, including some **Synology DSM** hosts, the kernel does not expose `/sys/.../powercap` even though `/dev/cpu/0/msr` is available.

In that case, uncomment both blocks in `compose.yaml`:

```yaml
cap_add:
  - SYS_RAWIO

devices:
  - /dev/cpu/0/msr:/dev/cpu/0/msr:r
```

The Linux `msr` driver requires **`CAP_SYS_RAWIO`** to open `/dev/cpu/*/msr`, even when the container process runs as root. `privileged: true` is not required: enable only `SYS_RAWIO` together with the MSR device when this fallback is actually needed.

PowerWatch always prefers `powercap`. MSR is used only as a fallback when no RAPL `powercap` domain is discovered. Only one CPU device is required: `MSR_PKG_ENERGY_STATUS` is **package-wide**, so `/dev/cpu/0/msr` and `/dev/cpu/1/msr` must not be summed.

For hardware and Docker details, see [`DOCKER.md`](DOCKER.md).

## Native compilation

Docker remains the recommended deployment method for PowerWatch. Native compilation is intended for advanced users who want to run the binaries directly on their Linux distribution.

### Requirements

You need:

- **Git**;
- a recent stable **Rust** and **Cargo** toolchain;
- a working C build toolchain (`gcc` or `clang`, linker, `make`, etc.).

Rust can be installed using the official [`rustup`](https://rustup.rs/) method.

Common system prerequisite examples:

```bash
# Debian / Ubuntu
sudo apt install build-essential pkg-config git curl dmidecode

# Fedora
sudo dnf group install "Development Tools"
sudo dnf install git pkgconf-pkg-config curl dmidecode

# Arch Linux / Garuda
sudo pacman -S --needed base-devel git curl dmidecode
```

### Build PowerWatch

Clone the repository and build the complete workspace in release mode:

```bash
git clone https://github.com/Aerya/PowerWatch.git
cd PowerWatch
cargo build --release --workspace
```

The four executables are generated under `target/release/`:

```text
powerwatch
powerwatch-tui
powerwatch-web
powerwatch-hub
```

### Run the binaries

CLI:

```bash
./target/release/powerwatch
./target/release/powerwatch --json
```

TUI:

```bash
./target/release/powerwatch-tui
```

Local Web UI with history:

```bash
./target/release/powerwatch-web   --host 127.0.0.1   --port 3000   --log   --history-interval 60
```

For a server/headless system, add `--nas-mode`. To listen on the LAN, use a private address or `0.0.0.0`, keeping in mind that PowerWatch **has no built-in authentication**.

Hub with its configuration and history files stored in the current directory:

```bash
./target/release/powerwatch-hub   --host 127.0.0.1   --port 3065   --config ./powerwatch-hub.json   --database ./powerwatch-hub.db
```

### Hardware access

When running natively, PowerWatch reads the hardware interfaces exposed by Linux directly, so there are no Docker bind mounts to configure.

- **NVIDIA**: NVIDIA Container Toolkit is not required for native execution. The NVIDIA driver/NVML only needs to work on the host; `nvidia-smi` can be used to verify it.
- **AMD / Intel**: available counters under `/sys` are read directly.
- **RAPL MSR**: if the `/dev/cpu/0/msr` fallback is required, the process must have the permissions/capabilities required by the kernel to open the device, including `CAP_SYS_RAWIO` depending on the host configuration.

> **Support:** native compilation is provided as an advanced option. Dependencies, permissions, hardware paths, and behavior can vary across distributions and kernels. Docker remains the reference deployment documented and tested by PowerWatch CI.

## GPU

### AMD

No additional Docker configuration is required. PowerWatch reads the Linux `amdgpu` counters exposed in `/sys`.

### Intel

PowerWatch first uses the `i915` / `xe` counters available through `hwmon`.

When they do not provide power telemetry but RAPL exposes an `uncore` subdomain, it can be used as the `gpu:intel:0` fallback.

### NVIDIA

NVIDIA support relies on **NVML**. The NVIDIA driver must work on the host and **NVIDIA Container Toolkit must be installed and configured for Docker**.

First verify the host driver:

```bash
nvidia-smi
```

Then verify/install NVIDIA Container Toolkit and configure the Docker runtime:

```bash
sudo nvidia-ctk runtime configure --runtime=docker
sudo systemctl restart docker
```

Before starting PowerWatch, validate GPU access from Docker:

```bash
docker run --rm --runtime=nvidia --gpus all ubuntu nvidia-smi
```

If this command shows the GPU, Docker/NVIDIA is correctly configured. You can then uncomment `NVIDIA_VISIBLE_DEVICES` / `NVIDIA_DRIVER_CAPABILITIES` and the `deploy.resources.reservations.devices` block in `compose.yaml`, then recreate PowerWatch:

```bash
docker compose up -d --force-recreate
```

If Docker reports:

```text
could not select device driver "nvidia" with capabilities: [[gpu]]
```

the failure happens **before PowerWatch starts**: Docker does not yet have a usable NVIDIA runtime. Bind-mounting `nvidia-smi` into the container does not fix this.

On Ubuntu/Debian, if `nvidia-container-toolkit` is not available from your currently configured repositories, follow NVIDIA's official installation instructions before running `nvidia-ctk`: [NVIDIA Container Toolkit — Install Guide](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html).

`NVIDIA_DRIVER_CAPABILITIES: utility` is sufficient for PowerWatch to access NVML; no graphics stack is required. Multiple GPUs and mixed AMD/Intel/NVIDIA systems are supported whenever the corresponding counters are readable.

## Disks, partitions, and storage

PowerWatch creates a power sensor only for each **physical disk**. Layers such as mdraid, LVM, and dm-crypt are not added as fake disks and therefore are not counted a second time.

When Linux can reconstruct the relationship, only chains that end in a **useful mounted filesystem** are displayed, for example:

```text
disk (sda) — sda1 → md0 [RAID1] → vg-data/lv-media [LVM] → /mnt/data [ext4]

disk (nvme0n1) — nvme0n1p2 → cryptroot [dm-crypt] → / [ext4]
```

Unmounted partitions are intentionally hidden. If a disk has no mounted partition, PowerWatch simply keeps the disk, its capacity, and its estimated power without adding an `[unmounted]` list. If some partitions are mounted and others are not, only the mounted ones are shown.

Each physical disk capacity is read from sysfs (`/sys/class/block/<device>/size`) and displayed in its enriched label. Internal identifiers remain stable (`disk:sda`, `disk:nvme0n1`, etc.) so mount changes do not break history or alerts.

### Disk power estimation

Disk values remain **estimates**, not direct electrical measurements. For generic profiles, PowerWatch uses the time the device was busy in `/proc/diskstats` between two samples and interpolates between an idle and an active value:

| Detected type | Idle | Active | Detection |
|---|---:|---:|---|
| Rotational HDD | 4 W | 8 W | `queue/rotational = 1` |
| Non-removable SSD | 0.5 W | 3 W | non-rotational and non-removable |
| Low-power flash (removable USB flash, eMMC/SD) | 0.2 W | 1.5 W | `removable = 1` or an `mmcblk*` device |
| NVMe generic fallback | 1 W | 6 W | `nvme*` device |

For example, an SSD busy for roughly 50% of the sampling interval is estimated halfway between 0.5 W and 3 W instead of immediately jumping to the full active value after any I/O. The ratio is clamped between 0 and 100%.

For NVMe drives, PowerWatch first tries to use `nvme-cli` together with the current power state exposed by sysfs. When available, it uses the maximum power advertised by the drive for the current power state. If that information is unavailable, PowerWatch falls back to the generic NVMe profile above.

Removable-media detection depends on what the kernel exposes, so a USB enclosure may be classified as HDD, SSD, or flash according to `rotational` and `removable`. Confidence remains **Estimated** in every case.

## Web UI

<p align="center">
  <img src="docs/images/PowerWatch-WebUI.png" alt="PowerWatch Web UI: sensors, total power usage, and history">
</p>

The main page displays:

- total power consumption;
- every detected sensor;
- the **measured / estimated** distinction;
- historical data;
- average, minimum, maximum, and energy;
- preset and custom time ranges;
- enriched disk labels including capacity;
- CPU model with a CPU Benchmark / PassMark link;
- an editable instance name reused as the suggested name when adding the instance to PowerWatch Hub;
- FR / EN language selection.

The Docker image automatically starts PowerWatch with history enabled and NAS/headless mode.

## History

Data is stored in SQLite in the persistent `./data` volume.

Current retention:

- raw measurements: **30 days**;
- 15-minute aggregates: **from 30 days to 1 year**;
- 1-hour aggregates: **beyond 1 year**.

Long time ranges are aggregated server-side to avoid unnecessarily sending thousands of points to the Web UI.

## Alerts and notifications

<p align="center">
  <img src="docs/images/Image_ALERTS.png" alt="PowerWatch Alerts">
</p>

The `/alerts` page lets you create persistent rules for:

- total power;
- CPU;
- all GPUs;
- a specific GPU (`gpu:nvidia:0`, `gpu:amd:0`, `gpu:intel:0`, etc.);
- RAM;
- a specific disk.

Each rule can define:

- a threshold in watts;
- a minimum over-threshold duration;
- enabled / disabled state;
- recovery notification.

Available notification methods:

- **Discord** via incoming webhook;
- **Apprise API**.

Alert settings are stored in the persistent Docker volume together with history data.

## CLI and TUI in Docker

The Web UI is the main service, but the image also includes the CLI and TUI.

### Snapshot

```bash
docker exec powerwatch powerwatch
```

### JSON

```bash
docker exec powerwatch powerwatch --json
```

### CLI history

```bash
docker exec powerwatch powerwatch history --since 1h
```

### TUI

<p align="center">
  <img src="docs/images/Image_TUI.png" alt="PowerWatch TUI">
</p>

```bash
docker exec -it powerwatch powerwatch-tui
```

Running the CLI or TUI with `docker exec` does not interrupt the Web UI.

## Check detected sensors

```bash
docker exec powerwatch powerwatch --json
```

Example GPU identifiers:

```text
gpu:nvidia:0
gpu:nvidia:1
gpu:amd:0
gpu:intel:0
```

To check RAPL directly inside the container:

```bash
docker exec powerwatch sh -c \
  'find /host-sys-virtual/powercap/intel-rapl -name energy_uj -o -name name 2>/dev/null'
```


## PowerWatch Hub — multiple machines, one dashboard

PowerWatch can also run in **Hub** mode. Each machine keeps its local PowerWatch instance and hardware collection; the Hub simply queries their HTTP APIs and groups them into a single Web UI.

<p align="center">
  <img src="docs/images/PowerWatch-Hub-Overview.png" alt="PowerWatch Hub: detailed view of federated instances">
</p>

<p align="center">
  <img src="docs/images/PowerWatch-Hub.png" alt="PowerWatch Hub: wide view of the total and federated history">
</p>

The Hub provides:

- a **global infrastructure total** labeled **“Measured + estimated”**;
- one collapsible card per machine with CPU, GPU, RAM, and disks;
- **online / stale / offline / disabled** states;
- adding, testing, enabling, and removing instances directly from the Web UI;
- the ability to exclude a machine from the global total;
- federated SQLite history, stored every 60 seconds by default;
- automatic recovery of aggregated history already available on the nodes (up to 400 days);
- a federated graph with the **global total + one colored line per instance**;
- compact node cards with CPU / GPU / RAM / disk summaries and measured / estimated colors;
- an FR / EN interface visually aligned with the main PowerWatch Web UI.

The global total adds up the node values, **both measured and estimated**. It is not a wall-outlet power measurement, and nominal PSU capacity is never included.

The Hub requires **no `/sys` access, no `pid: host`, and no privileged host access**. It only needs network access to the private URLs of the PowerWatch instances.

### Start the Hub

```bash
mkdir -p hub-data
docker compose -f compose.hub.yaml pull
docker compose -f compose.hub.yaml up -d
```

By default, the Hub dashboard listens on:

```text
http://127.0.0.1:3065
```

To expose it on the private LAN:

```env
POWERWATCH_HUB_BIND_IP=192.168.0.50
POWERWATCH_HUB_PORT=3065
```

Machines can then be added from the Web UI using their PowerWatch URL, for example:

```text
Garuda       http://192.168.0.53:3064
LincStation  http://192.168.0.196:3064
DockerLab    http://192.168.0.2:3064
```

Configuration is stored in `hub-data/powerwatch-hub.json` and federated history in `hub-data/powerwatch-hub.db`.

> On Hub startup and when an instance is added or re-enabled, PowerWatch Hub retrieves the aggregated history available from the node through `/api/history/range` (up to 400 days). The import is idempotent, so the same points are not duplicated in SQLite.

## Security

PowerWatch needs access to several host hardware interfaces to measure components.

The supplied Compose file notably uses:

- read-only `/sys`;
- read-only `/sys/devices/virtual` for RAPL;
- `pid: host` for some host information;
- a read-only container filesystem;
- `no-new-privileges:true`.

The Web UI currently has **no built-in authentication or HTTPS**. Do not expose it directly through a port forward, public tunnel, or Internet-facing reverse proxy without additional access protection.

## Attribution

Original project: [lnpotter/PowerWatch](https://github.com/lnpotter/PowerWatch)

Thanks to its author for the original project.

## License

See [`LICENSE`](LICENSE).
