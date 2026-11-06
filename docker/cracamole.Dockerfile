# cracamole: drop-in replacement for the official guacamole/guacd image.
# Validated on linux/amd64. Pin base images by digest before releasing.

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev build-base
WORKDIR /src
# Use the toolchain supplied by the Rust base image. Copying the local "stable"
# override would install a second toolchain (including rustfmt/clippy) in this layer.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p guacd

FROM alpine:3 AS runtime
# Fonts for terminal rendering: the bundled DejaVu Sans Mono covers Latin; CJK needs a fallback font.
RUN apk add --no-cache font-noto-cjk \
 && adduser -D -u 1000 guacd
COPY --from=build /src/target/release/guacd /usr/local/sbin/guacd
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 755 /usr/local/bin/entrypoint.sh
ENV CRACAMOLE_FONT_DIRS=/usr/share/fonts
USER guacd
EXPOSE 4822
HEALTHCHECK --interval=30s --timeout=3s CMD nc -z 127.0.0.1 4822 || exit 1
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
