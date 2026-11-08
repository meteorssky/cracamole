#!/usr/bin/env bash
# VNC verification loop: run the TigerVNC fixture (tests/vnc) on a remote Docker host, tunnel its
# ports to localhost, then run the Rust live tests and the browser tests against the locally built
# cracamole-web. The container is limited to 384 MiB / 200 processes and removed afterwards.
#   scripts/vnc_dev.sh root@10.0.20.130 [playwright args]
set -euo pipefail
export MSYS2_ENV_CONV_EXCL=CRACAMOLE_E2E_REMOTE_DIR MSYS_NO_PATHCONV=1
HOST=${1:?usage: vnc_dev.sh user@docker-host [playwright args]}; shift || true
ROOT=$(cd "$(dirname "$0")/.." && pwd); cd "$ROOT"
DIR=/opt/cracamole-vnc-dev
NAME=cracamole-vnc-dev
ssh "$HOST" "rm -rf $DIR && mkdir -p $DIR"
tar -C tests/vnc -c Dockerfile vnc.sh | ssh "$HOST" "tar -x --no-same-owner -C $DIR"
ssh "$HOST" "cd $DIR && docker build -q -t $NAME . >/dev/null && docker rm -f $NAME >/dev/null 2>&1; \
  docker run -d --name $NAME --memory 384m --pids-limit 200 \
  -p 127.0.0.1:15901:5901 -p 127.0.0.1:15902:5902 $NAME >/dev/null"
ssh -o ExitOnForwardFailure=yes -N -L 15901:127.0.0.1:15901 -L 15902:127.0.0.1:15902 "$HOST" & TUNNEL=$!
export CARGO_INCREMENTAL=0
cargo build -p cracamole-web
DATA=$(mktemp -d)
CRACAMOLE_ADMIN_PASSWORD=fixture-admin-password-2026 CRACAMOLE_HTTP_BIND=127.0.0.1:18081 CRACAMOLE_DATA_DIR="$DATA" \
  LOG_LEVEL=debug target/debug/cracamole-web > "$DATA/web.log" 2>&1 & WEB=$!
trap 'kill $WEB $TUNNEL 2>/dev/null; ssh "$HOST" "docker rm -f $NAME >/dev/null 2>&1; rm -rf $DIR"; echo "--- web log tail ---"; tail -20 "$DATA/web.log"' EXIT
sleep 3
export CRACAMOLE_VNC_RESIZE_CMD="ssh -o BatchMode=yes $HOST docker exec $NAME sh -c 'DISPLAY=:1 xrandr --fb 1024x768'"
cargo test -p proto-vnc --test live_vnc -- --ignored --test-threads=1
cd tests/e2e
CRACAMOLE_URL=http://127.0.0.1:18081/ CRACAMOLE_VNC_HOST=127.0.0.1 CRACAMOLE_VNC_PORT=15901 \
  npx playwright test vnc.spec.js "$@"
