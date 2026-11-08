#!/bin/sh
# :1 -> 5901, 800x600, VNC password authentication
# :2 -> 5902, 640x480, no authentication
set -eu
printf '%s\n' "$VNC_PASSWORD" | vncpasswd -f > /tmp/passwd
chmod 600 /tmp/passwd
Xvnc :1 -geometry 800x600 -depth 24 -rfbport 5901 -rfbauth /tmp/passwd -SecurityTypes VncAuth -AlwaysShared &
Xvnc :2 -geometry 640x480 -depth 24 -rfbport 5902 -SecurityTypes None -AlwaysShared &
sleep 2
for d in 1 2; do
  DISPLAY=:$d xsetroot -solid '#336699'
  DISPLAY=:$d xterm -geometry 40x10+20+20 -bg black -fg white -fa DejaVuSansMono -fs 14 &
done
wait
