# RAPL permissions (CPU sensor)

The CPU power sensor reads energy counters from
`/sys/class/powercap/intel-rapl:*`. On most distros, that path is only
readable by root, so `powerwatch` will report the CPU as "permission
denied" the first time you run it as a regular user.

## Fix: install the udev rule

```bash
sudo ./scripts/setup-udev.sh
```

This installs a udev rule that makes the RAPL counters world-readable
whenever the `powercap` subsystem loads (at boot, or right away if
`udevadm` is available). It only affects *read* access to energy counters -
nothing else changes, and no data is exposed beyond how much power your CPU
is drawing.

After running it, `powerwatch` should work without `sudo`. If it doesn't
right away, a reboot will apply the rule.

## Alternative: just use sudo

If you'd rather not install a system-wide rule, you can always run:

```bash
sudo powerwatch
```

This works with any `powerwatch` command (`--watch`, `--log`, etc.), but
you'll need `sudo` every time.

## Why this permission model exists

RAPL exposes real-time CPU energy consumption, which some distros treat as
sensitive (in theory, very sensitive attackers could try to infer
information about what's running from power fluctuations - a "side
channel"). That's why it defaults to root-only instead of world-readable.
The udev rule above accepts that small theoretical risk in exchange for
convenience, which is a reasonable trade-off for a desktop machine you
control. If that trade-off doesn't sit right with you, stick to `sudo`.
