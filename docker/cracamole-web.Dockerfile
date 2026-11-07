# syntax=docker/dockerfile:1
# Dependencies are compiled in their own layer (cargo-chef). That layer is rebuilt only when
# Cargo.toml / Cargo.lock change, so a code or front-end change recompiles just the workspace crates.
FROM rust:1-alpine AS chef
RUN apk add --no-cache musl-dev build-base  && cargo install cargo-chef --locked
WORKDIR /src

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY web ./web
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
# Empty means "use every core"; set JOBS=2 on small CI runners to bound memory.
ARG JOBS=
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p cracamole-web ${JOBS:+--jobs "$JOBS"} --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY web ./web
RUN cargo build --release --locked -p cracamole-web ${JOBS:+--jobs "$JOBS"}  && mkdir -p /out && cp target/release/cracamole-web /out/cracamole-web

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
