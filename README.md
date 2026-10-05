# PowerWatch

<p align="center">
  <img src="docs/images/powerwatch-aerya-logo.png" alt="PowerWatch logo" width="720">
</p>

A lightweight, cross-platform tool for measuring and estimating power consumption of PC components in real-time. Provides three interfaces: a CLI for scripts and one-off checks, a terminal dashboard for live monitoring, and a browser-based dashboard with charts and history.

## Overview

PowerWatch reports real or estimated power draw (in watts) for each major component: CPU, GPU (NVIDIA/AMD), RAM, and disk. Every reading is tagged as either `(measured)` or `(estimated)`, and the total reflects the least-trustworthy input, so you always know what you can rely on.

Linux is the primary, fully tested platform. Windows and macOS are supported in code but haven't been validated on real hardware yet.



> ## 🇫🇷 À propos de ce fork
>
> Ce fork de **PowerWatch** ajoute principalement :
> - le support **Docker** avec images multi-architecture **amd64 / arm64** publiées via GitHub Actions ;
> - une gestion des **alertes depuis la WebUI**, avec règles persistantes et notifications **Discord / Apprise** ;
> - une WebUI accessible sur le **réseau local** grâce à une adresse d’écoute configurable ;
> - une **WebUI bilingue français / anglais**, avec détection automatique au premier accès et mémorisation du choix ;
> - une détection RAPL plus robuste en conteneur, avec montage direct du powercap de l’hôte ;
> - la détection et le suivi de **tous les disques physiques**, avec exclusion des couches virtuelles, RAID/LVM et des pseudo-périphériques eMMC `boot`/`rpmb` ;
> - un **mode NAS/headless** qui désactive les suggestions desktop inutiles ;
> - un historique SQLite persistant avec vue **Live** et périodes personnalisables en heures, jours, semaines, mois ou années ;
> - l’agrégation automatique de l’historique avec **moyenne / minimum / maximum / énergie (kWh)** pour garder l’interface légère sur les longues périodes.
>
> ⚠️ La WebUI ne disposant d’aucune authentification, elle est destinée à un **usage LAN uniquement** et ne doit pas être exposée directement sur Internet.
>
> ## 🇬🇧 About this fork
>
> This fork of **PowerWatch** mainly adds:
> - **Docker** support with multi-architecture **amd64 / arm64** images published through GitHub Actions;
> - **Web UI alert management**, with persistent rules and **Discord / Apprise** notifications;
> - LAN access to the Web UI through a configurable bind address;
> - a **bilingual French / English Web UI**, with automatic first-visit detection and persistent language selection;
> - more robust RAPL discovery in containers, including a direct host powercap mount;
> - detection and monitoring of **all physical disks**, excluding virtual/RAID/LVM layers and eMMC `boot`/`rpmb` pseudo devices;
> - a **NAS/headless mode** that disables desktop-only energy suggestions;
> - persistent SQLite history with a **Live** view and arbitrary ranges in hours, days, weeks, months or years;
> - automatic history aggregation with **average / minimum / maximum / energy (kWh)** to keep long-range queries lightweight.
>
> ⚠️ Since the Web UI does not provide authentication, it is intended for **LAN-only use** and should not be exposed directly to the Internet.



## What It Measures

| Component | Source | Kind |
|-----------|--------|------|
| CPU | RAPL (`/sys/class/powercap`) | Measured |
| GPU (NVIDIA) | NVML | Measured |
| GPU (AMD) | hwmon (`power1_average`) | Measured |
| RAM | Heuristic (GB used × watts/GB) | Estimated |
| Disk | Idle/active watts by type (NVMe uses drive's own power-state table when `nvme-cli` is available) | Estimated |
| Total | Sum of the above | Mixed (tagged) |

Sensors that aren't available on your machine are silently omitted. Intel iGPUs aren't supported yet.

On NVMe drives, if `nvme-cli` is installed and the drive is readable, PowerWatch uses the manufacturer's declared max power per power state (`nvme id-ctrl`) rather than a generic guess. It falls back to the heuristic automatically when `nvme-cli` isn't available or the drive isn't NVMe.

## Install

You need a recent Rust toolchain (stable, 1.75+). No system libraries are required: SQLite is bundled, and the NVML binding loads dynamically only when the NVIDIA driver is present.

```bash
cargo build --workspace --release
```

Binaries are placed under `target/release/`: `powerwatch` (CLI), `powerwatch-tui` (terminal dashboard), and `powerwatch-web` (browser dashboard). Copy them anywhere on your `PATH` if you like:

```bash
sudo cp target/release/powerwatch target/release/powerwatch-tui target/release/powerwatch-web /usr/local/bin/
```

### CPU readings need a one-time setup

RAPL is root-only by default. Either run the CLI with `sudo`, or install the included udev rule:

```bash
sudo ./scripts/setup-udev.sh
```

See `docs/setup-udev.md` for what the rule does and why the restriction exists.

## Usage

```bash
# One-off snapshot
powerwatch

# JSON output (for scripts or pipes)
powerwatch --json

# Live terminal dashboard, refreshing once a second
powerwatch-tui

# Browser dashboard (local only, no auth)
powerwatch-web               # http://127.0.0.1:3000
powerwatch-web --port 4000   # custom port
powerwatch-web --log         # also write to the history database
```

### History logging

The CLI can record readings to a local SQLite database for later analysis:

```bash
# Log for 60 seconds
powerwatch --log --duration 60s

# Also accepts "5m", "1h", or a bare number of seconds
powerwatch --log --duration 5m
```

History lives at `~/.local/share/powerwatch/history.db`. You can inspect it directly:

```bash
sqlite3 ~/.local/share/powerwatch/history.db "SELECT * FROM readings ORDER BY ts DESC LIMIT 20;"
```

Query averaged readings from a logged session:

```bash
powerwatch history --since 1h
powerwatch history --since 30m
```

### Alerts

Get notified when a component stays above a threshold for a configured duration. Works in both the CLI and TUI:

```bash
# CLI: needs --watch so there's something to sustain
powerwatch --watch --alert-above 50 --alert-for 30s \
  --alert-run "notify-send 'PowerWatch' 'total power is high'"

# TUI: always live, no --watch needed
powerwatch-tui --alert-component cpu --alert-above 20 --alert-for 10s
```

| Flag | Meaning |
|------|---------|
| `--alert-above <watts>` | Enable the alert (required) |
| `--alert-component <name>` | Which sensor to watch (`cpu`, `gpu`, `ram`, `disk`, or `total`) |
| `--alert-for <duration>` | How long the threshold must be sustained (`10s`, `2m`, default: immediate) |
| `--alert-run <command>` | Shell command to run once when the alert fires |

The alert fires **once per excursion** above the threshold (not once per second) so a hook won't spam you. It can fire again after dropping back below and crossing upward a second time. Without `--alert-run`, the CLI prints `🔔 alert active` each cycle and the TUI shows a red banner.

## Live Dashboard (TUI)

![TUI](docs/images/Image_TUI.png)

```bash
powerwatch-tui
```

Opens a full-screen dashboard updating once per second: the total at top, one block per sensor below (current watts, confidence, a sparkline of the last minute, and min/max), and a footer listing available keys.

| Key | Action |
|-----|--------|
| `q` | Quit |
| `p` | Pause/resume sampling (freezes the last frame, shows `PAUSED`) |
| `l` | Start/stop logging to the history database |
| `s` | Open/close the suggestions panel |

If every sensor fails mid-session, the total line reports the failure directly instead of going quiet.

### Suggestions Panel (TUI)

The TUI can evaluate power-draw patterns and surface energy-saving suggestions. Press `s` to open the panel:

| Key | Action |
|-----|--------|
| `s` | Toggle suggestions panel |
| `Enter` | Confirm the first actionable suggestion |
| `y` / `Y` | Apply the selected action |
| `n` / `N` | Dismiss the suggestion |
| `s` again | Close the panel |

Each suggestion carries a confirmation token to prevent accidental or remote-triggered changes, you must explicitly confirm before any action runs.

## Web Dashboard

![WEB](docs/images/Image_WEB.png)

```bash
powerwatch-web                             # http://127.0.0.1:3000
powerwatch-web --port 4000
powerwatch-web --log
powerwatch-web --history-interval 300
powerwatch-web --nas-mode
```

The Web UI is available in **English and French**. On the first visit it follows the browser language when French is detected, otherwise it falls back to English. The selected language can be changed at any time with the `🇫🇷 FR` / `🇬🇧 EN` controls and is remembered in the browser.

The dashboard provides live readings plus historical views. Presets include `1 h`, `24 h`, `7 d`, `30 d`, and `1 y`, with custom ranges in hours, days, weeks, months, or years.

For the selected period it displays average, minimum, maximum, and energy in kWh. Long periods are aggregated server-side.

The Docker image already starts with `--log --history-interval 60 --nas-mode`.

### History retention

- raw readings: 30 days;
- 15-minute rollups: 30 days to 1 year;
- 1-hour rollups: older than 1 year, retained long-term.

API endpoints:

```text
GET  /api/snapshot
GET  /api/history?since=10m
GET  /api/history/range?amount=7&unit=days
GET  /api/suggestions
POST /api/suggestions/apply
GET  /api/processes/top
```

The dashboard is for the local machine or a trusted private LAN only. It has no built-in authentication or HTTPS.


## Web Alerts and Notifications

The Web UI includes a dedicated **Alerts** page at `/alerts`.

![Web Alerts](docs/images/Image_ALERTS.png)

Alerts are evaluated continuously by the Web server and can monitor `total`, `cpu`, `gpu`, `ram`, or any discovered disk sensor such as `disk:sda` / `disk:nvme0n1`. Each rule can define an enable/disable state, a threshold in watts, a sustained duration before firing, and an optional recovery notification.

Alert settings are persisted in `~/.local/share/powerwatch/alerts.json`. In Docker, `HOME=/data`, so the supplied persistent data volume keeps both history and alert configuration across container updates.

Notification methods:

- **Discord** through a standard incoming webhook;
- **Apprise API**, through a saved configuration endpoint such as `/notify/KEY`, or through the stateless `/notify/` endpoint with one or more Apprise notification URLs.

The Alerts page includes test buttons for Discord, Apprise, or all configured providers. Notifications fire once per threshold excursion and can fire again after the reading recovers and later crosses the threshold again.

The Docker image includes `curl`, used for outgoing Discord and Apprise HTTP notifications. When running `powerwatch-web` natively, install `curl` on the host if you want Web notifications.

### Docker CLI and TUI

The Docker image also contains the original CLI and TUI alongside the Web UI:

```bash
docker exec powerwatch powerwatch
docker exec powerwatch powerwatch --json
docker exec powerwatch powerwatch history --since 1h
docker exec -it powerwatch powerwatch-tui
```

The Web UI remains the container entrypoint, so launching the CLI or TUI with `docker exec` does not interrupt it.

## Energy-Saving Suggestions

PowerWatch can detect sustained high power draw and propose actions to reduce it. Every action requires explicit confirmation through a modal dialog with a one-time token; nothing is applied automatically.

### Implemented Suggestions

| Suggestion | Condition | Action | Safety |
|------------|-----------|--------|--------|
| **Switch to power saver** | Total > 15W for 3s, or CPU > 8W for 3s | Change power profile to `power-saver` | Modifies system profile |
| **Enable screensaver** | Total > 20W for 30s (idle heuristic) | Suspend screensaver (`xdg-screensaver suspend`) | Safe, reversible |
| **Show top processes** | CPU > 8W for 10s | Display top 5 CPU processes in a modal | Read-only, no system changes |
| **Show sleep timer** | System appears idle | Display current sleep configuration | Read-only, no system changes |

### Power Profile Commands

| Profile | Linux | macOS | Windows |
|---------|-------|-------|---------|
| Power Saver | `powerprofilesctl set power-saver` | `pmset -a lowpowermode 1` | `powercfg /setactive SCHEME_MAX` |
| Balanced | `powerprofilesctl set balanced` | `pmset -a lowpowermode 0` | `powercfg /setactive SCHEME_BALANCED` |
| Performance | `powerprofilesctl set performance` | `pmset -a lowpowermode 0` | `powercfg /setactive SCHEME_MIN` |

### Security Notes

The web dashboard can execute system commands from a browser click. While it binds only to `127.0.0.1` and has no authentication, consider these risks:

- **Confirmation tokens** mitigate CSRF. Each token is single-use and server-generated.
- **Never expose the port on the public Internet**. LAN-only use is acceptable on a trusted private network, but do not publish it through a public reverse proxy, tunnel, or router port-forwarding.
- **Double confirmation** is shown in the browser (`confirm()` dialog) in addition to the token.

Use the web dashboard only on trusted local machines.

### Platform-Specific Sensor Sources

**Linux**: direct filesystem reads (`/sys/class/powercap` for RAPL, `hwmon` for AMD GPU, NVML for NVIDIA, `iostat`/`nvme-cli` for disk).

**Windows**: LibreHardwareMonitor WMI provider when available (real CPU RAPL/MSR), falling back to `GetSystemTimes` × assumed TDP for CPU; NVML unchanged; RAM uses `GlobalMemoryStatusEx`; disk uses the PDH performance counter API.

**macOS**: `powermetrics` for CPU/GPU (requires `sudo`), `vm_stat` + `sysctl hw.memsize` for RAM, `iostat` for disk. CPU and GPU are measured; disk and RAM are estimated.

## Development

```bash
cargo test --workspace   # unit tests - no real hardware required
cargo clippy --workspace # lints
cargo fmt                # formatting
cargo build --release    # optimized binaries
```

Hardware-facing sensors (RAPL, NVML, hwmon) take a path or handle as a constructor argument, so tests exercise parsing and calculation logic against fixture data rather than live hardware. Manual validation on real hardware is still recommended before trusting numbers on an unfamiliar machine.

## Windows Support (Experimental)

Building on Windows:

```powershell
cargo build --release
.\target\release\powerwatch.exe
.\target\release\powerwatch-tui.exe
```

Most likely failure points, in rough order of confidence:
1. LibreHardwareMonitor WMI query (`powerwatch-core/src/sensors/windows/lhm.rs`) — COM/WMI via the `wmi` crate is the least verifiable piece blind.
2. Disk sensor (`powerwatch-core/src/sensors/windows/disk.rs`) — PDH API behavior can vary.
3. CPU fallback path — `GetSystemTimes` semantics (kernel time includes idle) are easy to get subtly wrong.
4. RAM sensor — simplest of the four, most likely to just work.

## macOS Support (Experimental)

CPU and GPU are **measured** via `powermetrics` (Apple reports real wattage). The trade-off: `powermetrics` always requires root, so the whole process must run under `sudo`:

```bash
sudo ./target/release/powerwatch
sudo ./target/release/powerwatch-tui
```

Most likely failure points:
1. Disk sensor (`powerwatch-core/src/sensors/macos/disk.rs`) — `iostat` column layout depends on disk count.
2. CPU/GPU sensor — the plist parsing is simple and fixture-tested, but the exact `powermetrics` invocation is unverified.
3. RAM sensor — `vm_stat` "available" memory is an approximation; numbers should be in the right ballpark but may not match Activity Monitor exactly.

## License

[MIT](https://github.com/lnpotter/PowerWatch/blob/main/LICENSE)
## Docker notes

This fork adds Docker deployment for Linux hosts, robust RAPL access, Linux multi-disk discovery, persistent multi-year history, a custom README logo, and explicit LAN-only deployment guidance.

**Security:** PowerWatch has no authentication. Keep it on a trusted private LAN only. Do not expose it with a public reverse proxy, Cloudflare Tunnel, or router port-forwarding.

See [DOCKER.md](DOCKER.md) for deployment details.
