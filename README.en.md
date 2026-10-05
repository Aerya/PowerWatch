# PowerWatch

<p align="center">
  <img src="docs/images/powerwatch-aerya-logo.png" alt="PowerWatch" width="720">
</p>

<p align="center">
  <a href="README.md">Français</a> · <strong>English</strong>
</p>

**PowerWatch** monitors the power consumption of a Linux machine from Docker and exposes it through a Web UI with history, alerts, and several measured or estimated hardware sources.

This fork primarily targets **servers, mini PCs, Linux desktops and Docker hosts**. The deployment documented here is **Docker-only**.

> **Security:** the Web UI has no built-in authentication. Use it only on a **trusted private LAN**. Do not expose it directly to the Internet.

## Features

- Real-time Web UI with historical views.
- Persistent SQLite history with long-range aggregation.
- Alerts managed from the Web UI.
- **Discord** and **Apprise API** notifications.
- Multi-GPU **NVIDIA / AMD / Intel** discovery.
- **Intel RAPL `uncore` fallback** for some iGPUs without i915/xe hwmon power telemetry.
- Monitoring of all **physical disks** without double-counting RAID/LVM layers.
- Best-effort display of associated **partitions, mount points, filesystems, mdraid, LVM and dm-crypt**.
- CLI and TUI included in the Docker image.
- Multi-architecture **amd64 / arm64** GHCR images.
- **French / English** Web UI.

## Measurements

| Component | Source | Type |
|---|---|---|
| CPU | Linux RAPL | Measured |
| NVIDIA GPU | NVML | Measured |
| AMD GPU | `amdgpu` hwmon | Measured |
| Intel GPU | `i915` / `xe` hwmon or RAPL `uncore` | Measured |
| RAM | Heuristic | Estimated |
| Disks | Activity + disk type | Estimated |
| Total | Sum of available sensors | Mixed |

Unavailable sensors are simply omitted.

On some Intel systems, PowerWatch can use the RAPL `uncore` subdomain as iGPU power. In that case, CPU power is derived from the package minus `uncore` so the iGPU is not counted twice.

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

To make it reachable from your private LAN, edit `.env` and set the **private IPv4 address of the host**:

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

For detailed hardware and Docker notes, see [`DOCKER.md`](DOCKER.md).

## GPU support

### AMD

No additional Docker configuration is required. PowerWatch reads Linux `amdgpu` counters exposed through `/sys`.

### Intel

PowerWatch first uses `i915` / `xe` power telemetry exposed through hwmon.

When no usable hwmon power counter is available but RAPL exposes an `uncore` subdomain, that domain can be used as the `gpu:intel:0` fallback.

### NVIDIA

The NVIDIA driver and **NVIDIA Container Toolkit** must be installed on the host.

The NVIDIA section is already present in `compose.yaml` but commented out so the same file can also run on machines without NVIDIA. Uncomment the NVIDIA lines in the Compose file and restart:

```bash
docker compose up -d
```

Multiple GPUs and mixed AMD/Intel/NVIDIA systems are supported whenever the corresponding telemetry is readable.

## Disks, partitions and storage topology

PowerWatch creates a power sensor only for each **physical disk**.

Layers such as mdraid, LVM and dm-crypt are not added as extra disks and therefore are not counted twice.

When Linux can reconstruct the relationship, labels may look like:

```text
disk (sda) — sda1 → md0 [RAID1] → vg-data/lv-media [LVM] → /mnt/data [ext4]

disk (nvme0n1) — nvme0n1p2 → cryptroot [dm-crypt] → / [ext4]
```

An unmounted partition may also be shown as `[unmounted]`.

Internal sensor identifiers remain stable (`disk:sda`, `disk:nvme0n1`, etc.) so history and alert rules are not broken by mount changes.

## Web UI

<p align="center">
  <img src="docs/images/Image_WEB.png" alt="PowerWatch Web UI">
</p>

The main dashboard shows:

- total power;
- every detected sensor;
- **measured / estimated** confidence;
- historical views;
- average, minimum, maximum and energy;
- preset and custom time ranges;
- enriched disk labels;
- FR / EN language selection.

The Docker image starts PowerWatch with history logging and NAS/headless mode enabled.

## History

Data is stored in SQLite inside the persistent `./data` directory.

Current retention:

- raw readings: **30 days**;
- 15-minute rollups: **30 days to 1 year**;
- 1-hour rollups: **older than 1 year**.

Long ranges are aggregated server-side to avoid sending unnecessary thousands of points to the Web UI.

## Alerts and notifications

<p align="center">
  <img src="docs/images/Image_ALERTS.png" alt="PowerWatch Alerts">
</p>

The `/alerts` page can create persistent rules for:

- total power;
- CPU;
- aggregate GPU power;
- a specific GPU (`gpu:nvidia:0`, `gpu:amd:0`, `gpu:intel:0`, etc.);
- RAM;
- a specific disk.

Each rule can define:

- a watt threshold;
- a minimum sustained duration;
- enabled / disabled state;
- optional recovery notification.

Notification providers:

- **Discord** incoming webhook;
- **Apprise API**.

Alert settings are stored in the same persistent Docker data directory as the history database.

## CLI and TUI in Docker

The Web UI is the primary service, but the image also includes the CLI and TUI.

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

Starting the CLI or TUI with `docker exec` does not interrupt the Web UI.

## Verify detected sensors

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

To inspect RAPL directly from the container:

```bash
docker exec powerwatch sh -c \
  'find /host-sys-virtual/powercap/intel-rapl -name energy_uj -o -name name 2>/dev/null'
```

## Persistent data

The Compose file mounts:

```text
./data → /data/.local/share/powerwatch
```

This directory contains files such as:

```text
history.db
alerts.json
```

Keep `./data` when updating the container.

## Update

```bash
docker compose pull
docker compose up -d
docker image prune -f
```

## Security

PowerWatch needs access to several host hardware interfaces.

The supplied Compose file notably uses:

- read-only `/sys`;
- read-only `/sys/devices/virtual` for RAPL;
- `pid: host` for some host information;
- a read-only container filesystem;
- `no-new-privileges:true`.

The Web UI currently provides **no built-in authentication or HTTPS**. Do not expose it directly through a public port-forward, tunnel or Internet-facing reverse proxy without additional access protection.

## Platform supported by this fork

The documentation and published images from this fork target **Linux + Docker**.

The historical PowerWatch codebase still contains parts inherited from the original project for other platforms, but **Windows and macOS are outside the supported/documented scope of this fork**.

## Docker image

Image:

```text
ghcr.io/aerya/powerwatch:latest
```

Published architectures:

```text
linux/amd64
linux/arm64
```

Builds and images are produced by **GitHub Actions**.

## Attribution

This repository is a fork maintained by **Aerya**.

Original project: [lnpotter/PowerWatch](https://github.com/lnpotter/PowerWatch)

Thanks to its author for the original project.

## License

See [`LICENSE`](LICENSE).
