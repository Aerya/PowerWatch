# PowerWatch

<p align="center">
  <img src="docs/images/powerwatch-aerya-logo.png" alt="PowerWatch" width="720">
</p>

<p align="center">
  <a href="README.md">Français</a> · <strong>English</strong>
</p>

**PowerWatch** monitors the power consumption of a Linux machine from Docker and displays it in a Web UI, with history, alerts, and several measured or estimated hardware sources.

This fork is primarily designed for **servers, mini PCs, Linux desktop machines, and Docker hosts**. The deployment documented here is **Docker-only**.

> **Security:** the Web UI has no built-in authentication. Use it only on a **trusted private LAN**. Do not expose it directly to the Internet.

## Features

- Real-time Web UI with historical views.
- Persistent SQLite history with long-term aggregation.
- Alerts configurable from the Web UI.
- **Discord** and **Apprise API** notifications.
- Multi-GPU **NVIDIA / AMD / Intel** detection.
- **Intel RAPL `uncore` fallback** for some iGPUs without i915/xe hwmon counters.
- Monitoring of all **physical disks** without double-counting RAID/LVM layers.
- Best-effort display of associated **partitions, mount points, filesystems, mdraid, LVM, and dm-crypt**.
- CLI and TUI available in the Docker image.
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

Unavailable sensors are simply ignored.

On some Intel systems, PowerWatch can use the RAPL `uncore` subdomain as the iGPU power reading. In that case, CPU power is calculated from the package value minus `uncore` so the iGPU is not counted twice.

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

For hardware and Docker details, see [`DOCKER.md`](DOCKER.md).

## GPU

### AMD

No additional Docker configuration is required. PowerWatch reads the Linux `amdgpu` counters exposed in `/sys`.

### Intel

PowerWatch first uses the `i915` / `xe` counters available through `hwmon`.

When they do not provide power telemetry but RAPL exposes an `uncore` subdomain, it can be used as the `gpu:intel:0` fallback.

### NVIDIA

The NVIDIA driver and **NVIDIA Container Toolkit** must be installed on the host.

The NVIDIA block is already present in `compose.yaml`, but commented out so the same file also works on machines without NVIDIA. Uncomment the indicated NVIDIA lines in the Compose file, then restart:

```bash
docker compose up -d
```

Multiple GPUs and mixed AMD/Intel/NVIDIA systems are supported whenever the corresponding counters are readable.

## Disks, partitions, and storage

PowerWatch creates a power sensor only for each **physical disk**.

Layers such as mdraid, LVM, and dm-crypt are not added as fake disks and therefore are not counted a second time.

When Linux can reconstruct the relationship, the display may look like:

```text
disk (sda) — sda1 → md0 [RAID1] → vg-data/lv-media [LVM] → /mnt/data [ext4]

disk (nvme0n1) — nvme0n1p2 → cryptroot [dm-crypt] → / [ext4]
```

An unmounted partition may also appear with `[unmounted]`.

Internal identifiers remain stable (`disk:sda`, `disk:nvme0n1`, etc.) so mount changes do not break history or alerts.

## Web UI

<p align="center">
  <img src="docs/images/Image_WEB.png" alt="PowerWatch Web UI">
</p>

The main page displays:

- total power consumption;
- every detected sensor;
- the **measured / estimated** distinction;
- historical data;
- average, minimum, maximum, and energy;
- preset and custom time ranges;
- enriched disk labels;
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
