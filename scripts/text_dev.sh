#!/usr/bin/env bash
# Isolated SSH/Telnet encoding tests using the locally built Rust application.
# scripts/text_dev.sh root@docker-host [Playwright arguments]
set -euo pipefail
export MSYS_NO_PATHCONV=1
HOST=${1:?usage: text_dev.sh user@docker-host [Playwright arguments]}; shift
ROOT=$(cd "$(dirname "$0")/.." && pwd); cd "$ROOT"
node -e 'const net = require("node:net"); for (const port of [18082,26022,26023]) { const s=net.connect(port,"127.0.0.1"); s.on("connect",()=>{console.error("Test port already in use:",port); process.exitCode=1; s.destroy();}); s.on("error",()=>{}); }'
NAME=cracamole-text-$(date +%s)-$$
DATA=$(mktemp -d)
WEB= TUNNEL= DIR=
cleanup() {
  local result=$?
  trap - EXIT
  [ -z "$WEB" ] || kill "$WEB" 2>/dev/null || true
  [ -z "$TUNNEL" ] || kill "$TUNNEL" 2>/dev/null || true
  ssh "$HOST" "docker rm -f '$NAME' >/dev/null 2>&1 || true" || true
  if [[ "$DIR" =~ ^/tmp/cracamole-text\.[A-Za-z0-9]+$ ]]; then
    ssh "$HOST" "test -d '$DIR' && rm -r -- '$DIR'" || true
  fi
  echo "Application log: $DATA/web.log"
  exit "$result"
}
trap cleanup EXIT
ssh -o BatchMode=yes -o ConnectTimeout=5 "$HOST" 'docker ps; df -h /; free -m'
DIR=$(ssh "$HOST" 'mktemp -d /tmp/cracamole-text.XXXXXXXX')
[[ "$DIR" =~ ^/tmp/cracamole-text\.[A-Za-z0-9]+$ ]] || exit 1
tar -C tests/e2e -c backend.Dockerfile backend.sh |
  ssh "$HOST" "tar -x --no-same-owner -C '$DIR'"
ssh "$HOST" "docker build -q -f '$DIR/backend.Dockerfile' -t cracamole-e2e-backend '$DIR' && docker run -d --name '$NAME' --memory 128m --pids-limit 64 -p 127.0.0.1:26022:22 -p 127.0.0.1:26023:23 cracamole-e2e-backend"
ssh -o ExitOnForwardFailure=yes -N -L 26022:127.0.0.1:26022 -L 26023:127.0.0.1:26023 "$HOST" & TUNNEL=$!
export CARGO_INCREMENTAL=0
if [ "${CRACAMOLE_TEXT_SKIP_BUILD:-0}" != 1 ]; then
  cargo build --locked -p cracamole-web
fi
APP=target/debug/cracamole-web
[ ! -f "$APP.exe" ] || APP="$APP.exe"
# Windows locks a running executable. Run a copy so other checks can rebuild target/debug.
cp "$APP" "$DATA/cracamole-web.exe"
CRACAMOLE_ADMIN_PASSWORD=fixture-admin-password-2026 CRACAMOLE_HTTP_BIND=127.0.0.1:18082 CRACAMOLE_DATA_DIR="$DATA" \
  "$DATA/cracamole-web.exe" > "$DATA/web.log" 2>&1 & WEB=$!
cd tests/e2e
CRACAMOLE_URL=http://127.0.0.1:18082/ CRACAMOLE_E2E_SSH="$HOST" \
  CRACAMOLE_E2E_BACKEND_CONTAINER="$NAME" npx playwright test encoding.spec.js \
    --output "../../target/$NAME" --reporter=list "$@"
