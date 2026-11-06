# cracamole: drop-in replacement for the official guacamole/guacd image.
# NOTE: draft, not yet built in CI. Pin base images by digest before releasing.

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev build-base
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
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
