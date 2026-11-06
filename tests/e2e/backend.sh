#!/bin/sh
set -eu
telnetd -p 23 -l /bin/login
exec /usr/sbin/sshd -D -e
