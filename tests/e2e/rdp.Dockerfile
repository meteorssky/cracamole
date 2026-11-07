# Real RDP server for end-to-end tests: xrdp + Xorg session running xterm.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends xrdp xorgxrdp xterm dbus-x11 x11-utils netcat-openbsd xclip ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd -m -s /bin/bash tester \
 && echo 'tester:tester-pw' | chpasswd \
 && printf '#!/bin/sh\nexec xterm -geometry 80x24+20+20 -fa Monospace -fs 14 -bg black -fg white\n' > /etc/xrdp/startwm.sh \
 && chmod 755 /etc/xrdp/startwm.sh
COPY rdp.sh /usr/local/bin/rdp.sh
RUN chmod 755 /usr/local/bin/rdp.sh
EXPOSE 3389
ENTRYPOINT ["/usr/local/bin/rdp.sh"]
