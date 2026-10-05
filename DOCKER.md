# PowerWatch Docker deployment

PowerWatch has no authentication. Keep it on a trusted private LAN only; do not expose it through a public reverse proxy, tunnel, or router port-forwarding.

## Quick start / Démarrage rapide

PowerWatch uses a **single `compose.yaml`**.

```bash
docker compose up -d
```

The image defaults already enable:

```text
--host 0.0.0.0
--port 3000
--log
--history-interval 60
--nas-mode
```

The container mounts host `/sys` read-only for general hardware discovery and also mounts `/sys/devices/virtual` at `/host-sys-virtual` so CPU RAPL remains visible inside Docker. It uses the host PID namespace and persists SQLite history and alert configuration in `./data`.

### 🇫🇷 CPU / RAPL dans Docker

Le simple montage `/sys:/sys:ro` ne suffit pas sur toutes les distributions : certains hôtes exposent bien RAPL sur la machine mais le sous-arbre `powercap` n'apparaît pas au même endroit dans le conteneur.

Le Compose fourni monte donc aussi :

```text
/sys/devices/virtual:/host-sys-virtual:ro
```

et définit :

```text
POWERWATCH_POWERCAP_PATH=/host-sys-virtual/powercap/intel-rapl
```

Ce montage utilise un chemin parent générique présent sur Linux. Si RAPL est disponible, PowerWatch récupère la mesure CPU. Sur certains Intel, si aucun compteur i915/xe hwmon n'est exposé mais qu'un sous-domaine RAPL `uncore` existe, PowerWatch l'utilise comme mesure de l'iGPU. Dans ce cas, la valeur CPU est calculée comme `package - uncore` afin de ne pas compter deux fois l'iGPU dans le total. S'il n'y a pas de RAPL exploitable, PowerWatch continue normalement sans capteur CPU mesuré.

### 🇬🇧 CPU / RAPL in Docker

The plain `/sys:/sys:ro` mount is not sufficient on every distribution: some hosts expose RAPL correctly on the host while the `powercap` subtree is not visible at the same location inside the container.

The supplied Compose therefore also mounts:

```text
/sys/devices/virtual:/host-sys-virtual:ro
```

and sets:

```text
POWERWATCH_POWERCAP_PATH=/host-sys-virtual/powercap/intel-rapl
```

This uses a generic Linux parent path. When RAPL is available, PowerWatch restores measured CPU power. On some Intel systems, if no i915/xe hwmon counter is exposed but a RAPL `uncore` subdomain exists, PowerWatch uses it as measured iGPU power. In that case CPU is calculated as `package - uncore` so the iGPU is not counted twice in the total. If usable RAPL telemetry is absent, PowerWatch continues normally without a measured CPU sensor.

### 🇫🇷 GPU AMD et Intel

Rien à modifier dans le Compose.

PowerWatch lit directement la télémétrie de puissance exposée par Linux dans `/sys/class/hwmon` :

- AMD : `amdgpu` (`power1_average` / `power1_input`) ;
- Intel : `i915` / `xe` (`power1_*` ou `energy1_input`).

Si Intel n'expose aucun compteur hwmon mais fournit un sous-domaine RAPL `uncore`, PowerWatch utilise automatiquement ce compteur comme fallback `gpu:intel:0`. Si plusieurs GPU AMD/Intel sont présents, les compteurs hwmon restent prioritaires et chaque GPU lisible est suivi séparément.

### 🇬🇧 AMD and Intel GPUs

Nothing needs to be changed in the Compose file.

PowerWatch reads the power telemetry exposed by Linux directly through `/sys/class/hwmon`:

- AMD: `amdgpu` (`power1_average` / `power1_input`);
- Intel: `i915` / `xe` (`power1_*` or `energy1_input`).

If Intel exposes no hwmon power counter but provides a RAPL `uncore` subdomain, PowerWatch automatically uses it as the `gpu:intel:0` fallback. When multiple AMD/Intel GPUs are present, hwmon telemetry remains preferred and each readable GPU is monitored separately.

## NVIDIA

NVIDIA requires two host-side prerequisites that Docker cannot provide automatically:

1. the NVIDIA driver;
2. NVIDIA Container Toolkit.

The NVIDIA block is already present in the **same `compose.yaml`**, but commented by default so PowerWatch can start normally on machines without NVIDIA.

### 🇫🇷 Activation NVIDIA

Dans `compose.yaml`, décommentez entièrement les blocs `environment:` et `deploy:` de la section **NVIDIA GPU(S)**, puis :

```bash
docker compose up -d
```

`NVIDIA_VISIBLE_DEVICES: all` expose tous les GPU NVIDIA et la capability `utility` fournit NVML, utilisée par PowerWatch pour lire leur consommation réelle.

Une machine mixte fonctionne de la même façon :

- AMD + NVIDIA : AMD est détecté via `/sys`, activez simplement le bloc NVIDIA ;
- Intel iGPU + NVIDIA : Intel est détecté via `/sys`, activez simplement le bloc NVIDIA ;
- plusieurs AMD, Intel ou NVIDIA : chaque GPU est enregistré séparément.

### 🇬🇧 Enabling NVIDIA

In `compose.yaml`, fully uncomment the `environment:` and `deploy:` blocks under **NVIDIA GPU(S)**, then run:

```bash
docker compose up -d
```

`NVIDIA_VISIBLE_DEVICES: all` exposes every NVIDIA GPU and the `utility` capability provides NVML, which PowerWatch uses for measured GPU power.

Mixed-vendor systems work the same way:

- AMD + NVIDIA: AMD is detected through `/sys`; simply enable the NVIDIA block;
- Intel iGPU + NVIDIA: Intel is detected through `/sys`; simply enable the NVIDIA block;
- multiple AMD, Intel or NVIDIA GPUs: every GPU is stored separately.

## Verify detected hardware / Vérifier le matériel détecté

```bash
docker exec powerwatch powerwatch --json
```

GPU sensor names are stable per vendor/index, for example:

```text
gpu:nvidia:0
gpu:nvidia:1
gpu:amd:0
gpu:intel:0
```

The generic alert target `gpu` remains available and represents the sum of all currently readable GPU sensors.

## Disk topology / Topologie des disques

PowerWatch continues to create power sensors only for physical disks, so mdraid, LVM and dm-crypt layers are **not** counted as additional disks. On Linux, the displayed disk label is enriched best-effort from `/sys/class/block` and the host mount table visible through `/proc/1/mountinfo`. No additional Compose mount is required because the supplied Compose already uses `pid: host` and mounts `/sys` read-only.

Example / Exemple:

```text
disk (sda) — sda1 → md0 [RAID1] → vg-data [LVM] → /mnt/data [ext4]
disk (nvme0n1) — nvme0n1p2 → cryptroot [dm-crypt] → / [ext4]
```

Unmounted partitions can also be shown as `[unmounted]`. Unsupported or ambiguous storage stacks simply fall back to the stable physical disk label instead of inventing a relationship.

## Web UI language

The Web UI is available in **English and French**. Language selection is handled entirely in the browser: French is selected automatically on the first visit when the browser language is French, English is the fallback, and the selected language is stored in browser `localStorage`.

## Web alerts and notifications

Open `/alerts` from the PowerWatch Web UI to create persistent alert rules.

The configuration is stored under the same persistent Docker data mount as the history database:

```text
/data/.local/share/powerwatch/alerts.json
```

Each rule can monitor the total draw or an individual discovered sensor, set a threshold in watts, require the threshold to remain exceeded for a chosen number of seconds, and optionally send a recovery notification.

Notification providers:

- **Discord:** paste a standard Discord incoming webhook URL;
- **Apprise API:** use either a saved configuration endpoint (`http://apprise:8000/notify/KEY`) or the stateless endpoint (`http://apprise:8000/notify/`) with Apprise URLs entered in the Web UI.

The page provides test buttons for both providers. The Docker image includes `curl` for outgoing notification requests.

Alert and notification settings contain secrets such as webhook URLs. Keep the PowerWatch data directory private and do not expose the unauthenticated Web UI outside a trusted LAN.

## CLI and TUI in Docker

The image contains `powerwatch-web`, `powerwatch`, and `powerwatch-tui`. The Web UI starts by default; the other interfaces can be opened in the running container:

```bash
docker exec powerwatch powerwatch
docker exec powerwatch powerwatch --json
docker exec powerwatch powerwatch history --since 1h
docker exec -it powerwatch powerwatch-tui
```

## History retention

- raw readings: 30 days;
- 15-minute rollups: from 30 days to 1 year;
- 1-hour rollups: older than 1 year, kept long-term.

Maintenance runs automatically while logging is enabled. The WebUI can query arbitrary hours, days, weeks, months, or years.

## Multi-architecture image

GitHub Actions publishes `linux/amd64` and `linux/arm64` images to `ghcr.io/aerya/powerwatch`.

## Attribution

Fork maintained by **Aerya**.

Thanks to [lnpotter/PowerWatch](https://github.com/lnpotter/PowerWatch) for the original project.
