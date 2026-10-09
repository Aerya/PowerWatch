# PowerWatch Docker deployment

PowerWatch authentication is optional and disabled by default. Keep an unauthenticated instance on a trusted private LAN only. See **Integrated authentication / Authentification intégrée** below before using a reverse proxy.

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

## Integrated authentication / Authentification intégrée

First authenticated start / Premier démarrage authentifié:

```dotenv
POWERWATCH_AUTH_ENABLED=true
POWERWATCH_AUTH_SETUP_TOKEN=a-long-random-secret-at-least-16-characters
```

Run `docker compose up -d`, open the Web UI, and create the single administrator account with the setup token. Then remove `POWERWATCH_AUTH_SETUP_TOKEN` and recreate the container. Keep `POWERWATCH_AUTH_ENABLED=true`.

The persistent bind mount stores `auth.json` under `./data`; Docker recreation therefore preserves the account, server-side sessions, and hashed integration tokens. The clear API token is only returned once when it is created.

For PowerWatch Hub, enter a read-only token from the PowerWatch **Security** page in the instance's optional API-token field. Dockge-Enhanced must send the same kind of token as `Authorization: Bearer <token>`.

Hub administration can be protected separately with:

```dotenv
POWERWATCH_HUB_ADMIN_TOKEN=a-random-secret-with-at-least-24-characters
```

When configured, use this Bearer token for `/api/hub/nodes`, `/api/hub/nodes/:id`, and `/api/hub/test`. The Hub dashboard keeps it only for the current browser tab. A stored instance token is preserved for same-URL updates, but it is cleared on a URL change unless a token is explicitly supplied for the new destination.

Behind HTTPS, the proxy must overwrite and send `X-Forwarded-Proto: https`; this makes PowerWatch set the session cookie's `Secure` attribute. Health remains public at `/api/health`.

## SMBIOS / RAM / PSU

Docker masks `/sys/firmware` inside containers by default, so the generic `/sys:/sys:ro` bind is not enough for SMBIOS inventory.

The reference Compose explicitly adds:

```text
/sys/firmware:/host-sys-firmware:ro
```

and:

```text
POWERWATCH_DMI_TABLE_PATH=/host-sys-firmware/dmi/tables/DMI
```

PowerWatch parses the host DMI table directly from that read-only path to obtain populated SMBIOS Type 17 memory devices and Type 39 power-supply records. This does **not** require `privileged: true`, `/dev/mem`, or extra capabilities.

If the host kernel does not expose the raw DMI table, PowerWatch keeps the Linux `/proc/meminfo` fallback for usable memory and can still try `dmidecode`. The UI deliberately distinguishes:

- installed RAM from SMBIOS vs usable RAM from Linux;
- SMBIOS unavailable vs a valid SMBIOS table that simply contains no Type 39 PSU record.

On ordinary desktop PCs, an absent Type 39 record is common and does not indicate a PowerWatch error.

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

Ce montage utilise un chemin parent générique présent sur Linux. Si RAPL est disponible, PowerWatch récupère la mesure CPU. Sur certains Intel, si aucun compteur i915/xe hwmon n'est exposé mais qu'un sous-domaine RAPL `uncore` existe, PowerWatch l'utilise comme mesure de l'iGPU. Dans ce cas, la valeur CPU est calculée comme `package - uncore` afin de ne pas compter deux fois l'iGPU dans le total.

Sur certains noyaux, notamment **Synology DSM / Gemini Lake**, `powercap` peut être absent alors que `/dev/cpu/0/msr` existe. PowerWatch peut alors lire directement les compteurs RAPL MSR. Le Compose de référence contient les blocs suivants, commentés par défaut :

```yaml
cap_add:
  - SYS_RAWIO

devices:
  - /dev/cpu/0/msr:/dev/cpu/0/msr:r
```

Le pilote Linux `msr` exige `CAP_SYS_RAWIO` même pour root dans le conteneur ; le simple mapping du device ne suffit pas et provoque `Operation not permitted`. Il n'est pas nécessaire de passer PowerWatch en `privileged: true`.

S'il n'y a ni `powercap` exploitable ni accès MSR autorisé, PowerWatch continue normalement sans capteur CPU mesuré.

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

This uses a generic Linux parent path. When RAPL is available, PowerWatch restores measured CPU power. On some Intel systems, if no i915/xe hwmon counter is exposed but a RAPL `uncore` subdomain exists, PowerWatch uses it as measured iGPU power. In that case CPU is calculated as `package - uncore` so the iGPU is not counted twice in the total.

On some kernels, notably **Synology DSM / Gemini Lake**, `powercap` may be absent while `/dev/cpu/0/msr` is available. PowerWatch can then read RAPL directly through MSR. The reference Compose includes these blocks, commented by default:

```yaml
cap_add:
  - SYS_RAWIO

devices:
  - /dev/cpu/0/msr:/dev/cpu/0/msr:r
```

The Linux `msr` driver requires `CAP_SYS_RAWIO` even for root inside the container; mapping the device alone is not enough and results in `Operation not permitted`. PowerWatch does not need `privileged: true`.

If neither usable `powercap` telemetry nor permitted MSR access is available, PowerWatch continues normally without a measured CPU sensor.

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

NVIDIA support relies on NVML and requires host-side setup that Docker cannot provide automatically. Having the proprietary NVIDIA driver installed is **not enough**: Docker must also have NVIDIA Container Toolkit installed and its runtime configured.

Official documentation: [NVIDIA Container Toolkit — Install Guide](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html).

### 🇫🇷 Activation NVIDIA

Vérifiez d'abord que le pilote fonctionne directement sur l'hôte :

```bash
nvidia-smi
```

Installez ensuite **NVIDIA Container Toolkit** si nécessaire. Sur Ubuntu/Debian, si le paquet n'est pas disponible dans vos dépôts actuels, ajoutez le dépôt officiel en suivant le guide NVIDIA lié ci-dessus.

Configurez ensuite explicitement Docker pour le runtime NVIDIA et redémarrez le daemon :

```bash
sudo nvidia-ctk runtime configure --runtime=docker
sudo systemctl restart docker
```

Avant même de démarrer PowerWatch, testez l'accès GPU avec un conteneur indépendant :

```bash
docker run --rm --runtime=nvidia --gpus all ubuntu nvidia-smi
```

Cette commande doit afficher le ou les GPU NVIDIA. Si elle échoue, corrigez d'abord la configuration NVIDIA/Docker.

En particulier, l'erreur :

```text
could not select device driver "nvidia" with capabilities: [[gpu]]
```

signifie que Docker ne dispose pas d'un runtime NVIDIA utilisable. Elle se produit **avant le démarrage de PowerWatch** et ne se corrige pas en montant manuellement `nvidia-smi` ou `/dev/nvidia*` dans le conteneur.

Une fois le test Docker fonctionnel, décommentez dans `compose.yaml` :

```yaml
environment:
  NVIDIA_VISIBLE_DEVICES: all
  NVIDIA_DRIVER_CAPABILITIES: utility

deploy:
  resources:
    reservations:
      devices:
        - driver: nvidia
          count: all
          capabilities: [gpu]
```

Puis recréez PowerWatch :

```bash
docker compose up -d --force-recreate
```

`NVIDIA_VISIBLE_DEVICES: all` rend tous les GPU disponibles au runtime et `NVIDIA_DRIVER_CAPABILITIES: utility` expose les bibliothèques/outils nécessaires à NVML. PowerWatch n'a pas besoin des capabilities graphiques `graphics` ou `display`.

Une machine mixte fonctionne de la même façon :

- AMD + NVIDIA : AMD est détecté via `/sys`, activez simplement NVIDIA comme ci-dessus ;
- Intel iGPU + NVIDIA : Intel est détecté via `/sys`, activez simplement NVIDIA comme ci-dessus ;
- plusieurs AMD, Intel ou NVIDIA : chaque GPU lisible est enregistré séparément.

### 🇬🇧 Enabling NVIDIA

First verify that the NVIDIA driver works directly on the host:

```bash
nvidia-smi
```

Then install **NVIDIA Container Toolkit** if needed. On Ubuntu/Debian, if the package is not available from your currently configured repositories, add NVIDIA's official repository by following the installation guide linked above.

Explicitly configure Docker for the NVIDIA runtime and restart the daemon:

```bash
sudo nvidia-ctk runtime configure --runtime=docker
sudo systemctl restart docker
```

Before starting PowerWatch, test GPU access with an independent container:

```bash
docker run --rm --runtime=nvidia --gpus all ubuntu nvidia-smi
```

This command must display the NVIDIA GPU(s). If it fails, fix the NVIDIA/Docker setup first.

In particular, this error:

```text
could not select device driver "nvidia" with capabilities: [[gpu]]
```

means Docker does not have a usable NVIDIA runtime. It happens **before PowerWatch starts** and is not fixed by manually bind-mounting `nvidia-smi` or `/dev/nvidia*` into the container.

Once the Docker test works, uncomment in `compose.yaml`:

```yaml
environment:
  NVIDIA_VISIBLE_DEVICES: all
  NVIDIA_DRIVER_CAPABILITIES: utility

deploy:
  resources:
    reservations:
      devices:
        - driver: nvidia
          count: all
          capabilities: [gpu]
```

Then recreate PowerWatch:

```bash
docker compose up -d --force-recreate
```

`NVIDIA_VISIBLE_DEVICES: all` makes every GPU available to the runtime and `NVIDIA_DRIVER_CAPABILITIES: utility` exposes the libraries/tools required for NVML. PowerWatch does not need the `graphics` or `display` capabilities.

Mixed-vendor systems work the same way:

- AMD + NVIDIA: AMD remains detected through `/sys`; enable NVIDIA as shown above;
- Intel iGPU + NVIDIA: Intel remains detected through `/sys`; enable NVIDIA as shown above;
- multiple AMD, Intel, or NVIDIA GPUs: every readable GPU is tracked separately.

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
