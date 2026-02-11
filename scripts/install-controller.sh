#!/usr/bin/env bash
#
# Pillar Controller installer — installs the controller on a Linux machine.
# Expects pre-built binary (via --binaries-dir or downloaded from GitHub releases).
#
# Usage:
#   sudo ./install-controller.sh --binaries-dir /path/to/binaries
#   sudo ./install-controller.sh --external-url http://1.2.3.4:50051
#
# Idempotent — safe to run multiple times.

set -euo pipefail

# ------------------------------------------------------------------------------
# Defaults
# ------------------------------------------------------------------------------

PILLAR_USER="pillar"
PILLAR_GROUP="pillar"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/pillar"
DB_DIR="/var/lib/pillar"
LOG_DIR="/var/log/pillar"

DB_PATH="$DB_DIR/controller.db"
GRPC_LISTEN="0.0.0.0:50051"
HTTP_LISTEN="0.0.0.0:8080"
RETENTION_DAYS=30
EXTERNAL_URL=""
BINARIES_DIR=""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

WARN_COUNT=0
FAIL_COUNT=0

info()  { echo -e "${BLUE}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; WARN_COUNT=$((WARN_COUNT + 1)); }
fail()  { echo -e "${RED}[FAIL]${NC}  $*"; FAIL_COUNT=$((FAIL_COUNT + 1)); }
die()   { echo -e "${RED}[FATAL]${NC} $*"; exit 1; }
section() { echo -e "\n${BLUE}--- $* ---${NC}"; }

# ------------------------------------------------------------------------------
# Argument parsing
# ------------------------------------------------------------------------------

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binaries-dir)    BINARIES_DIR="$2";   shift 2 ;;
        --db-path)         DB_PATH="$2";         shift 2 ;;
        --grpc-listen)     GRPC_LISTEN="$2";     shift 2 ;;
        --http-listen)     HTTP_LISTEN="$2";     shift 2 ;;
        --external-url)    EXTERNAL_URL="$2";    shift 2 ;;
        --retention-days)  RETENTION_DAYS="$2";  shift 2 ;;
        --help|-h)
            head -10 "$0" | tail -6
            exit 0
            ;;
        *)
            die "unknown argument: $1 (try --help)"
            ;;
    esac
done

if [[ -z "$BINARIES_DIR" ]]; then
    die "--binaries-dir is required (path to directory containing controller binary)"
fi

if [[ ! -f "$BINARIES_DIR/controller" ]]; then
    die "controller binary not found in $BINARIES_DIR"
fi

# Auto-detect external URL from public IP if not set
if [[ -z "$EXTERNAL_URL" ]]; then
    PUBLIC_IP=$(curl -sf --max-time 5 ifconfig.me 2>/dev/null || curl -sf --max-time 5 ipinfo.io/ip 2>/dev/null || true)
    if [[ -n "$PUBLIC_IP" ]]; then
        EXTERNAL_URL="http://${PUBLIC_IP}:50051"
        info "auto-detected external URL: $EXTERNAL_URL"
    fi
fi

# ------------------------------------------------------------------------------
# Phase 1: Preflight
# ------------------------------------------------------------------------------

section "Preflight checks"

if [[ $EUID -ne 0 ]]; then
    die "this script must be run as root (use sudo)"
fi

if [[ "$(uname -s)" != "Linux" ]]; then
    die "this script only supports Linux (detected: $(uname -s))"
fi
ok "Linux detected"

if ! command -v systemctl &>/dev/null; then
    die "systemd not found"
fi
ok "systemd available"

# ------------------------------------------------------------------------------
# Phase 2: System user and directories
# ------------------------------------------------------------------------------

section "Setting up system user and directories"

if getent group "$PILLAR_GROUP" &>/dev/null; then
    ok "group $PILLAR_GROUP exists"
else
    groupadd --system "$PILLAR_GROUP"
    ok "created group $PILLAR_GROUP"
fi

if id "$PILLAR_USER" &>/dev/null; then
    ok "user $PILLAR_USER exists"
else
    useradd --system --no-create-home --shell /usr/sbin/nologin --gid "$PILLAR_GROUP" "$PILLAR_USER"
    ok "created user $PILLAR_USER"
fi

DB_DIR_ACTUAL="$(dirname "$DB_PATH")"
for dir in "$CONFIG_DIR" "$LOG_DIR" "$DB_DIR_ACTUAL"; do
    mkdir -p "$dir"
    chown "$PILLAR_USER:$PILLAR_GROUP" "$dir"
    chmod 755 "$dir"
done
ok "directories ready"

# ------------------------------------------------------------------------------
# Phase 3: Install binary
# ------------------------------------------------------------------------------

section "Installing binary"

DST="$INSTALL_DIR/controller"
install -m 755 "$BINARIES_DIR/controller" "$DST"
ok "installed controller -> $DST"

# ------------------------------------------------------------------------------
# Phase 4: Write config
# ------------------------------------------------------------------------------

section "Writing configuration"

CONTROLLER_CONFIG="$CONFIG_DIR/controller.yaml"
if [[ -f "$CONTROLLER_CONFIG" ]]; then
    ok "config exists: $CONTROLLER_CONFIG (not overwriting)"
else
    cat > "$CONTROLLER_CONFIG" <<EOF
grpc_listen: "$GRPC_LISTEN"
http_listen: "$HTTP_LISTEN"
db_path: "$DB_PATH"
retention_days: $RETENTION_DAYS
external_url: "$EXTERNAL_URL"
EOF
    chown "$PILLAR_USER:$PILLAR_GROUP" "$CONTROLLER_CONFIG"
    chmod 644 "$CONTROLLER_CONFIG"
    ok "wrote $CONTROLLER_CONFIG"
fi

# ------------------------------------------------------------------------------
# Phase 5: Systemd service
# ------------------------------------------------------------------------------

section "Installing systemd service"

UNIT_FILE="/etc/systemd/system/controller.service"
cat > "$UNIT_FILE" <<EOF
[Unit]
Description=Pillar Controller
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$PILLAR_USER
Group=$PILLAR_GROUP
ExecStart=$INSTALL_DIR/controller
Restart=always
RestartSec=5
StartLimitBurst=5
StartLimitIntervalSec=60

Environment=PILLAR_CONTROLLER_CONFIG=$CONTROLLER_CONFIG
Environment=RUST_LOG=info

StandardOutput=journal
StandardError=journal
SyslogIdentifier=controller

LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF
ok "wrote $UNIT_FILE"

systemctl daemon-reload
systemctl enable controller 2>/dev/null
systemctl restart controller
sleep 2

if systemctl is-active --quiet controller; then
    ok "controller is running"
else
    fail "controller failed to start"
    journalctl -u controller --no-pager -n 20
fi

# ------------------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------------------

section "Installation complete"

echo ""
echo "  Binary:     $DST"
echo "  Config:     $CONTROLLER_CONFIG"
echo "  Database:   $DB_PATH"
echo "  gRPC:       $GRPC_LISTEN"
echo "  HTTP UI:    http://localhost:8080"
if [[ -n "$EXTERNAL_URL" ]]; then
echo "  External:   $EXTERNAL_URL"
fi
echo ""
echo "  Commands:"
echo "    journalctl -u controller -f"
echo "    curl http://localhost:8080/api/overview"
echo "    curl http://localhost:8080/api/nodes"
echo ""
if [[ -n "$EXTERNAL_URL" ]]; then
echo "  Add nodes:"
echo "    sudo ./install-node.sh --binaries-dir /path/to/binaries --controller-endpoint $EXTERNAL_URL"
fi
echo ""
