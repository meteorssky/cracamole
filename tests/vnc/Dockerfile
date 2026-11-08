# Real third-party VNC servers (TigerVNC) for interoperability checks. Test fixture only.
FROM alpine:3
RUN apk add --no-cache tigervnc xterm xsetroot xrandr xdotool font-dejavu ttf-dejavu
COPY vnc.sh /vnc.sh
RUN chmod +x /vnc.sh && adduser -D tester
USER tester
ENV VNC_PASSWORD=fixture1
EXPOSE 5901 5902
ENTRYPOINT ["/vnc.sh"]
