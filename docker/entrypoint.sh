#!/bin/sh
# Entrypoint compatible with the official guacamole/guacd image:
#   guacd -f -b 0.0.0.0 -L "${LOG_LEVEL:-${GUACD_LOG_LEVEL:-info}}" "$@"
# GUACD_LOG_LEVEL is the deprecated spelling of LOG_LEVEL.

if [ -n "$GUACD_LOG_LEVEL" ] && [ -z "$LOG_LEVEL" ]; then
    echo "WARNING: The GUACD_LOG_LEVEL environment variable is deprecated. Please use LOG_LEVEL instead." >&2
fi

exec "${GUACD_BIN:-/usr/local/sbin/guacd}" -f -b 0.0.0.0 -L "${LOG_LEVEL:-${GUACD_LOG_LEVEL:-info}}" "$@"
