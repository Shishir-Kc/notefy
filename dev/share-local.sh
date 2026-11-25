#!/usr/bin/env bash
# Local sharing test: mock B2 + the Worker (wrangler dev) + Notefy wired to them.
# Nothing touches your real ~/.config/notefy or Backblaze; all state lives in dev/.
#   dev/share-local.sh [vault]     default vault: dev/vault
# Ctrl+C (or closing Notefy) stops everything. Bucket files: dev/.local-b2/notefy/<vault>/...
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
VAULT=$(realpath -m "${1:-dev/vault}")
WORKER=http://localhost:8787
TOKEN=dev-token

[ -d backend/node_modules ] || (cd backend && npm install)
# the Worker's dev secrets (fake B2 keys: the mock doesn't check them)
printf 'B2_KEY_ID=dev\nB2_APP_KEY=dev\nAPI_TOKEN=%s\n' "$TOKEN" > backend/.dev.vars
# Notefy reads its cloud config from here instead of ~/.config
export XDG_CONFIG_HOME=$ROOT/dev/.config
mkdir -p "$XDG_CONFIG_HOME/notefy" "$VAULT"
printf '%s\n%s\n' "$WORKER" "$TOKEN" > "$XDG_CONFIG_HOME/notefy/cloud"

if curl -s -o /dev/null "$WORKER/"; then echo "port 8787 is already in use: stop the other wrangler dev first" >&2; exit 1; fi

# stop the mock and wrangler (and their children) however the script ends
trap 'trap - EXIT; kill 0' EXIT INT TERM
node dev/mock-b2.mjs dev/.local-b2 9000 2>&1 | sed 's/^/[b2] /' &
(cd backend && npx wrangler dev --port 8787 --var B2_ENDPOINT:http://127.0.0.1:9000) 2>&1 | sed 's/^/[worker] /' &
until curl -s -o /dev/null "$WORKER/"; do sleep 0.5; done

echo "Worker ready on $WORKER — opening Notefy on $VAULT (⋯ menu → Share…)"
cargo run -- "$VAULT"
