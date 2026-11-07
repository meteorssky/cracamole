#!/usr/bin/env bash
# One-command browser E2E on a remote Docker host (no local Docker needed).
#   scripts/e2e_remote.sh root@10.0.20.130 [playwright args...]
# Uses its own directory/compose project, so it never touches a running deployment.
set -euo pipefail
# Git Bash on Windows would rewrite /opt/... paths handed to native node/ssh.
export MSYS2_ENV_CONV_EXCL=CRACAMOLE_E2E_REMOTE_DIR MSYS_NO_PATHCONV=1
HOST=${1:?usage: e2e_remote.sh user@docker-host [playwright args]}; shift || true
DIR=${CRACAMOLE_E2E_REMOTE_DIR:-/opt/cracamole-e2e}
PORT=${CRACAMOLE_E2E_PORT:-18080}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

echo "== sync working tree (incl. uncommitted) to $HOST:$DIR"
ssh -o BatchMode=yes "$HOST" "rm -rf '$DIR' && mkdir -p '$DIR'"
git ls-files -co --exclude-standard -z | tar --null -T - -c | ssh "$HOST" "tar -x --no-same-owner -C '$DIR'"

COMPOSE="docker compose -p cracamole-e2e -f $DIR/docker/compose.e2e.yml"
TUNNEL=
cleanup() {
  [ -n "$TUNNEL" ] && kill "$TUNNEL" 2>/dev/null || true
  ssh "$HOST" "$COMPOSE down -v --remove-orphans >/dev/null 2>&1; rm -rf '$DIR'" || true
}
trap cleanup EXIT

echo "== build and start fixture (Rust app + sshd/telnetd)"
ssh "$HOST" "$COMPOSE build --build-arg JOBS=${CRACAMOLE_E2E_JOBS:-} && $COMPOSE up -d --wait" 2>&1 | tail -5

echo "== tunnel localhost:$PORT -> $HOST"
ssh -o ExitOnForwardFailure=yes -N -L "$PORT:127.0.0.1:18080" "$HOST" &
TUNNEL=$!
sleep 2

echo "== playwright"
cd tests/e2e
[ -d node_modules ] || npm ci
npx playwright install chromium firefox webkit >/dev/null
CRACAMOLE_URL="http://127.0.0.1:$PORT/" CRACAMOLE_E2E_SSH="$HOST" CRACAMOLE_E2E_REMOTE_DIR="$DIR" \
  npx playwright test "$@"
