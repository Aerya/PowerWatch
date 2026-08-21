#!/usr/bin/env bash
# Grants read access to the RAPL power counters under /sys/class/powercap,
# so powerwatch can read CPU energy without running as root every time.
#
# What it does: installs a udev rule that makes RAPL's sysfs files
# world-readable whenever the powercap subsystem shows up (at boot, or when
# the driver loads). This only affects read access to energy counters - it
# doesn't grant write access or touch anything else.
#
# Needs sudo, since it writes to /etc/udev/rules.d.

set -euo pipefail

RULE_FILE="/etc/udev/rules.d/99-powerwatch-rapl.rules"
RULE_CONTENT='SUBSYSTEM=="powercap", ACTION=="add", RUN+="/bin/chmod -R a+r /sys%p"'

if [ "$(id -u)" -ne 0 ]; then
    echo "this needs to write to /etc/udev/rules.d - re-run with sudo" >&2
    exit 1
fi

mkdir -p "$(dirname "$RULE_FILE")"
echo "$RULE_CONTENT" > "$RULE_FILE"
echo "wrote $RULE_FILE"

if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules
    udevadm trigger --subsystem-match=powercap
    echo "done - RAPL counters should be readable now, no sudo needed"
else
    echo "udevadm not found - the rule is written and will take effect on next boot"
fi

echo "(if you don't see a change right away, a reboot will apply the rule too)"
