# PowerWatch Docker / NAS deployment

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
