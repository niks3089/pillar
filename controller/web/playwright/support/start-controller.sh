#!/usr/bin/env bash
# Boots a throwaway pillar-controller for Playwright: fresh SQLite DB, known
# admin password + agent token, embedded UI built from this checkout.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WEB="$HERE/../.."
ROOT="$WEB/../.."

if [[ "${PILLAR_E2E_SKIP_BUILD:-0}" != "1" ]]; then
  (cd "$WEB" && npm run build >/dev/null)
  (cd "$ROOT" && cargo build -q -p pillar-controller)
fi

STATE="$(mktemp -d -t pillar-e2e-controller.XXXXXX)"
cat > "$STATE/controller.yaml" <<YAML
grpc_listen: "0.0.0.0:${PILLAR_E2E_GRPC_PORT:-15051}"
http_listen: "127.0.0.1:${PILLAR_E2E_HTTP_PORT:-18080}"
db_path: "$STATE/controller.db"
auth_token: "${PILLAR_E2E_AUTH_TOKEN:-pillar-e2e-token}"
YAML

export PILLAR_CONTROLLER_CONFIG="$STATE/controller.yaml"
export PILLAR_INITIAL_ADMIN_PASSWORD="${PILLAR_E2E_ADMIN_PASSWORD:-pillar-e2e-password}"
exec "$ROOT/target/debug/controller"
