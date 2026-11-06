# Test-only C oracle, pinned to the same source revision as the migration notes.
FROM debian:bookworm-slim AS build
ARG GUACAMOLE_SERVER_COMMIT=bb1ba8ed1414ec6a239cea265d4e9c7e23b9c6a3
RUN apt-get update && apt-get install -y --no-install-recommends \
    autoconf automake build-essential ca-certificates curl libtool pkg-config \
    libcairo2-dev libcjson-dev libjpeg62-turbo-dev libpango1.0-dev libpng-dev \
    libssh2-1-dev libssl-dev libtelnet-dev uuid-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
RUN curl -fsSL "https://codeload.github.com/apache/guacamole-server/tar.gz/${GUACAMOLE_SERVER_COMMIT}" \
    | tar -xz --strip-components=1
RUN autoreconf -fi \
 && ./configure --prefix=/opt/guacamole --disable-guacenc --disable-guaclog \
    --disable-kubernetes --without-rdp --without-vnc --without-pulse \
    CFLAGS="-O2 -Wno-error=deprecated-declarations" \
 && make -j2 \
 && make install

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libcairo2 libcjson1 libjpeg62-turbo libpango-1.0-0 libpangocairo-1.0-0 \
    libpng16-16 libssh2-1 libssl3 libtelnet2 libuuid1 netcat-openbsd \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /opt/guacamole /opt/guacamole
ENV LD_LIBRARY_PATH=/opt/guacamole/lib
USER 1000:1000
EXPOSE 4822
HEALTHCHECK --interval=1s --timeout=2s CMD nc -z 127.0.0.1 4822
ENTRYPOINT ["/opt/guacamole/sbin/guacd", "-f", "-b", "0.0.0.0", "-L", "info"]
