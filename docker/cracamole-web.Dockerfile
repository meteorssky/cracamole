# syntax=docker/dockerfile:1
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev build-base
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY web ./web
ARG TARGETARCH
RUN --mount=type=cache,id=cracamole-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cracamole-target-${TARGETARCH},target=/src/target \
    # Copied source mtimes may predate the cached Cargo fingerprints.
    # Refresh local sources so a changed Docker context cannot reuse a stale binary.
    find crates web -type f -exec touch {} + \
 && cargo build --release --locked -p cracamole-web --jobs 2 \
 && mkdir -p /out && cp target/release/cracamole-web /out/cracamole-web

FROM alpine:3
RUN apk add --no-cache font-noto-cjk \
 && adduser -D -u 1000 cracamole \
 && mkdir -p /data && chown cracamole:cracamole /data && chmod 700 /data
COPY --from=build /out/cracamole-web /usr/local/bin/cracamole-web
COPY LICENSE /usr/share/licenses/cracamole/LICENSE
COPY web/vendor/LICENSE web/vendor/NOTICE /usr/share/licenses/guacamole-common-js/
COPY crates/guac-terminal/assets/fonts/LICENSE-DejaVu /usr/share/licenses/DejaVu/LICENSE
ENV CRACAMOLE_HTTP_BIND=0.0.0.0:8080 CRACAMOLE_DATA_DIR=/data CRACAMOLE_FONT_DIRS=/usr/share/fonts
USER cracamole
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s CMD wget -q -O /dev/null http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["/usr/local/bin/cracamole-web"]
