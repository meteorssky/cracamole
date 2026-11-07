#!/bin/sh
set -eu
mkdir -p /var/run/xrdp /run/dbus
rm -f /var/run/xrdp/*.pid
xrdp-sesman
exec xrdp --nodaemon
