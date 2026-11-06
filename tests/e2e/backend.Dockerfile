FROM alpine:3.22
RUN apk add --no-cache openssh busybox-extras util-linux-login vim htop tmux \
 && adduser -D -s /bin/sh tester \
 && echo 'tester:tester-pw' | chpasswd \
 && ssh-keygen -A \
 && printf '\nPasswordAuthentication yes\nPermitRootLogin no\n' >> /etc/ssh/sshd_config
COPY backend.sh /usr/local/bin/backend.sh
RUN chmod 755 /usr/local/bin/backend.sh
ENTRYPOINT ["/usr/local/bin/backend.sh"]
