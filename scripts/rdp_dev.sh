#!/usr/bin/env bash
# Fast RDP dev loop: run only the xrdp fixture on a remote Docker host, tunnel it to
# localhost:13389 and run the locally built cracamole-web against it (no 40-minute image build).
#   scripts/rdp_dev.sh root@10.0.20.130 [playwright args]
set -euo pipefail
export MSYS2_ENV_CONV_EXCL=CRACAMOLE_E2E_REMOTE_DIR MSYS_NO_PATHCONV=1
HOST=${1:?usage: rdp_dev.sh user@docker-host [playwright args]}; shift || true
ROOT=$(cd "$(dirname "$0")/.." && pwd); cd "$ROOT"
DIR=/opt/cracamole-rdp-dev
ssh "$HOST" "rm -rf $DIR && mkdir -p $DIR/tests/e2e"
tar -C tests/e2e -c rdp.Dockerfile rdp.sh | ssh "$HOST" "tar -x --no-same-owner -C $DIR/tests/e2e"
ssh "$HOST" "cd $DIR/tests/e2e && docker build -q -f rdp.Dockerfile -t cracamole-rdp-dev . >/dev/null && docker rm -f cracamole-rdp-dev >/dev/null 2>&1; docker run -d --name cracamole-rdp-dev -p 127.0.0.1:13389:3389 cracamole-rdp-dev >/dev/null"
ssh -o ExitOnForwardFailure=yes -N -L 13389:127.0.0.1:13389 "$HOST" & TUNNEL=$!
export CARGO_INCREMENTAL=0
cargo build -p cracamole-web
DATA=$(mktemp -d)
CRACAMOLE_ADMIN_PASSWORD=fixture-admin-password-2026 CRACAMOLE_HTTP_BIND=127.0.0.1:18081 CRACAMOLE_DATA_DIR="$DATA" \
  LOG_LEVEL=debug target/debug/cracamole-web > "$DATA/web.log" 2>&1 & WEB=$!
trap 'kill $WEB $TUNNEL 2>/dev/null; ssh "$HOST" "docker rm -f cracamole-rdp-dev >/dev/null 2>&1; rm -rf $DIR"; echo "--- web log tail ---"; tail -20 "$DATA/web.log"' EXIT
sleep 3
cd tests/e2e
CRACAMOLE_URL=http://127.0.0.1:18081/ CRACAMOLE_RDP_HOST=127.0.0.1 CRACAMOLE_RDP_PORT=13389 CRACAMOLE_E2E_SSH="$HOST" \
  CRACAMOLE_E2E_RDP_CONTAINER=cracamole-rdp-dev npx playwright test -g "rdp:" "$@"
