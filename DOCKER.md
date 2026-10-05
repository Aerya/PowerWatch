# PowerWatch Docker deployment

PowerWatch has no authentication. Keep it on a trusted private LAN only; do not expose it through a public reverse proxy, tunnel, or router port-forwarding.

The image defaults already enable:

```text
--host 0.0.0.0
--port 3000
--log
--history-interval 60
--nas-mode
```

Therefore the provided `compose.yaml` does not need a `command:` block unless you want to override those defaults.

The container mounts the Intel RAPL tree directly at `/host-powercap`, mounts `/sys` read-only for hardware discovery, uses the host PID namespace, and persists SQLite history in `./data`.

> The supplied RAPL mount targets Intel/Linux hosts. Remove or adapt it on systems without `/sys/devices/virtual/powercap/intel-rapl`.

## Web UI language

The Web UI is available in **English and French**. Language selection is handled entirely in the browser: French is selected automatically on the first visit when the browser language is French, English is the fallback, and the selected language is stored in browser `localStorage`. No Docker environment variable or server-side configuration is required.

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
