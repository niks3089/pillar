#!/usr/bin/env bash
#
# Pillar Controller installer — installs the controller, Prometheus, and Grafana.
# Provisions dashboards and data sources so metrics work out of the box.
#
# Usage:
#   curl -sSL https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/install-controller.sh | sudo bash
#   sudo ./install-controller.sh --version 0.1.0
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
HTTP_PORT=8080
RETENTION_DAYS=30
EXTERNAL_URL=""
VERSION="latest"
GRAFANA_PORT=3000
PROMETHEUS_PORT=9091

S3_BASE="https://janus-meter.s3.eu-north-1.amazonaws.com/pillar"

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
        --version)         VERSION="$2";         shift 2 ;;
        --db-path)         DB_PATH="$2";         shift 2 ;;
        --grpc-listen)     GRPC_LISTEN="$2";     shift 2 ;;
        --http-listen)     HTTP_LISTEN="$2";     shift 2 ;;
        --external-url)    EXTERNAL_URL="$2";    shift 2 ;;
        --retention-days)  RETENTION_DAYS="$2";  shift 2 ;;
        --grafana-port)    GRAFANA_PORT="$2";    shift 2 ;;
        --prometheus-port) PROMETHEUS_PORT="$2";  shift 2 ;;
        --help|-h)
            head -12 "$0" | tail -8
            exit 0
            ;;
        *)
            die "unknown argument: $1 (try --help)"
            ;;
    esac
done

# Parse HTTP port from HTTP_LISTEN
HTTP_PORT="${HTTP_LISTEN##*:}"

# ==============================================================================
# Phase 0: Download controller binary from S3
# ==============================================================================

section "Downloading controller binary"

S3_PATH="${S3_BASE}/${VERSION}/pillar-controller-linux-amd64"
if [[ "$VERSION" != "latest" ]]; then
    S3_PATH="${S3_BASE}/v${VERSION}/pillar-controller-linux-amd64"
fi
DOWNLOAD_DIR=$(mktemp -d)
info "downloading from $S3_PATH ..."
if ! curl -sSfL "$S3_PATH" -o "$DOWNLOAD_DIR/controller"; then
    die "failed to download controller binary from $S3_PATH"
fi
chmod +x "$DOWNLOAD_DIR/controller"
ok "downloaded controller binary"

# Auto-detect external URL from public IP if not set
if [[ -z "$EXTERNAL_URL" ]]; then
    PUBLIC_IP=$(curl -sf --max-time 5 ifconfig.me 2>/dev/null || curl -sf --max-time 5 ipinfo.io/ip 2>/dev/null || true)
    if [[ -n "$PUBLIC_IP" ]]; then
        EXTERNAL_URL="http://${PUBLIC_IP}:50051"
        info "auto-detected external URL: $EXTERNAL_URL"
    fi
fi

ARCH="$(uname -m)"

# ==============================================================================
# Phase 1: Preflight
# ==============================================================================

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

# Detect package manager
if command -v apt-get &>/dev/null; then
    PKG_MANAGER="apt"
elif command -v dnf &>/dev/null; then
    PKG_MANAGER="dnf"
elif command -v yum &>/dev/null; then
    PKG_MANAGER="yum"
else
    die "no supported package manager found (need apt, dnf, or yum)"
fi
ok "package manager: $PKG_MANAGER"

# ==============================================================================
# Phase 2: System user and directories
# ==============================================================================

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
CERTS_DIR="$CONFIG_DIR/certs"
for dir in "$CONFIG_DIR" "$LOG_DIR" "$DB_DIR_ACTUAL" "$CERTS_DIR"; do
    mkdir -p "$dir"
    chown "$PILLAR_USER:$PILLAR_GROUP" "$dir"
    chmod 755 "$dir"
done
ok "directories ready"

# ==============================================================================
# Phase 3: Install controller binary
# ==============================================================================

section "Installing controller binary"

DST="$INSTALL_DIR/controller"
install -m 755 "$DOWNLOAD_DIR/controller" "$DST"
rm -rf "$DOWNLOAD_DIR"
ok "installed controller -> $DST"

# ==============================================================================
# Phase 4: Install Prometheus
# ==============================================================================

section "Installing Prometheus"

install_prometheus_binary() {
    local PROM_VERSION="2.53.3"
    local PROM_ARCH="$ARCH"
    [[ "$PROM_ARCH" == "x86_64" ]] && PROM_ARCH="amd64"
    [[ "$PROM_ARCH" == "aarch64" ]] && PROM_ARCH="arm64"

    local PROM_URL="https://github.com/prometheus/prometheus/releases/download/v${PROM_VERSION}/prometheus-${PROM_VERSION}.linux-${PROM_ARCH}.tar.gz"
    info "downloading Prometheus ${PROM_VERSION}..."

    local TMP_DIR
    TMP_DIR=$(mktemp -d)
    curl -sSL "$PROM_URL" | tar xz -C "$TMP_DIR" --strip-components=1
    install -m 755 "$TMP_DIR/prometheus" /usr/local/bin/prometheus
    install -m 755 "$TMP_DIR/promtool" /usr/local/bin/promtool
    rm -rf "$TMP_DIR"

    if ! id prometheus &>/dev/null; then
        useradd --system --no-create-home --shell /usr/sbin/nologin prometheus
    fi
    mkdir -p /var/lib/prometheus /etc/prometheus
    chown prometheus:prometheus /var/lib/prometheus

    ok "Prometheus ${PROM_VERSION} installed from binary"
}

if command -v prometheus &>/dev/null; then
    ok "Prometheus already installed ($(prometheus --version 2>&1 | head -1 || echo 'unknown'))"
else
    case "$PKG_MANAGER" in
        apt)
            apt-get update -qq 2>/dev/null || true
            if apt-get install -y -qq prometheus >/dev/null 2>&1 && command -v prometheus &>/dev/null; then
                ok "Prometheus installed via apt"
            else
                install_prometheus_binary
            fi
            ;;
        dnf|yum)
            if $PKG_MANAGER install -y prometheus >/dev/null 2>&1 && command -v prometheus &>/dev/null; then
                ok "Prometheus installed via $PKG_MANAGER"
            else
                install_prometheus_binary
            fi
            ;;
    esac
fi

# Write Prometheus config
PROM_CONFIG_DIR="/etc/prometheus"
mkdir -p "$PROM_CONFIG_DIR"

cat > "$PROM_CONFIG_DIR/prometheus.yml" <<EOF
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: 'pillar-controller'
    scrape_interval: 10s
    static_configs:
      - targets: ['localhost:${HTTP_PORT}']
    metrics_path: '/metrics'
EOF
ok "wrote $PROM_CONFIG_DIR/prometheus.yml (scraping localhost:${HTTP_PORT}/metrics)"

# Prometheus systemd service
PROM_BIN=$(command -v prometheus)
PROM_DATA="/var/lib/prometheus"
mkdir -p "$PROM_DATA"

# If installed via apt, configure via /etc/default/prometheus (uses $ARGS env var)
if [[ -f /etc/default/prometheus ]]; then
    cat > /etc/default/prometheus <<EOF
ARGS="--web.listen-address=0.0.0.0:${PROMETHEUS_PORT} --storage.tsdb.retention.time=30d"
EOF
    ok "wrote /etc/default/prometheus (port ${PROMETHEUS_PORT})"
elif systemctl list-unit-files prometheus.service &>/dev/null 2>&1 && \
     systemctl cat prometheus.service &>/dev/null 2>&1; then
    ok "using existing prometheus.service"
else
    # No existing service — write our own
    if ! id prometheus &>/dev/null; then
        useradd --system --no-create-home --shell /usr/sbin/nologin prometheus
    fi
    chown -R prometheus:prometheus "$PROM_DATA"

    cat > /etc/systemd/system/prometheus.service <<EOF
[Unit]
Description=Prometheus
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=prometheus
Group=prometheus
ExecStart=${PROM_BIN} \\
  --config.file=${PROM_CONFIG_DIR}/prometheus.yml \\
  --storage.tsdb.path=${PROM_DATA} \\
  --web.listen-address=0.0.0.0:${PROMETHEUS_PORT} \\
  --storage.tsdb.retention.time=30d
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
    ok "wrote prometheus.service"
fi

systemctl daemon-reload
systemctl enable prometheus 2>/dev/null
systemctl restart prometheus
sleep 1

if systemctl is-active --quiet prometheus; then
    ok "Prometheus running on port $PROMETHEUS_PORT"
else
    fail "Prometheus failed to start"
    journalctl -u prometheus --no-pager -n 5
fi

# ==============================================================================
# Phase 5: Install Grafana
# ==============================================================================

section "Installing Grafana"

if command -v grafana-server &>/dev/null || [[ -f /usr/sbin/grafana-server ]]; then
    ok "Grafana already installed"
else
    case "$PKG_MANAGER" in
        apt)
            apt-get install -y -qq apt-transport-https software-properties-common >/dev/null 2>&1 || true
            if [[ ! -f /etc/apt/keyrings/grafana.gpg ]]; then
                mkdir -p /etc/apt/keyrings
                curl -sSL https://apt.grafana.com/gpg.key | gpg --dearmor -o /etc/apt/keyrings/grafana.gpg
            fi
            if [[ ! -f /etc/apt/sources.list.d/grafana.list ]]; then
                echo "deb [signed-by=/etc/apt/keyrings/grafana.gpg] https://apt.grafana.com stable main" \
                    > /etc/apt/sources.list.d/grafana.list
            fi
            apt-get update -qq 2>/dev/null || true
            if apt-get install -y -qq grafana 2>&1; then
                ok "Grafana installed via apt"
            else
                fail "Grafana install failed"
            fi
            ;;
        dnf|yum)
            if [[ ! -f /etc/yum.repos.d/grafana.repo ]]; then
                cat > /etc/yum.repos.d/grafana.repo <<'REPO'
[grafana]
name=grafana
baseurl=https://rpm.grafana.com
repo_gpgcheck=1
enabled=1
gpgcheck=1
gpgkey=https://rpm.grafana.com/gpg.key
sslverify=1
sslcacert=/etc/pki/tls/certs/ca-bundle.crt
REPO
            fi
            $PKG_MANAGER install -y grafana >/dev/null 2>&1
            ok "Grafana installed via $PKG_MANAGER"
            ;;
    esac
fi

# ==============================================================================
# Phase 6: Configure Grafana
# ==============================================================================

section "Configuring Grafana"

GRAFANA_CONF="/etc/grafana/grafana.ini"
GRAFANA_PROV="/etc/grafana/provisioning"
GRAFANA_DASHBOARDS_DIR="/var/lib/grafana/dashboards/pillar"

# 6a: Patch grafana.ini — enable embedding, anonymous access, set port
# Uses sed with section-aware approach
if [[ -f "$GRAFANA_CONF" ]]; then
    # Server settings
    sed -i '/^\[server\]/,/^\[/ s/^;*\s*http_port\s*=.*/http_port = '"${GRAFANA_PORT}"'/' "$GRAFANA_CONF"
    sed -i '/^\[server\]/,/^\[/ s|^;*\s*root_url\s*=.*|root_url = %(protocol)s://%(domain)s:%(http_port)s/grafana/|' "$GRAFANA_CONF"
    sed -i '/^\[server\]/,/^\[/ s/^;*\s*serve_from_sub_path\s*=.*/serve_from_sub_path = true/' "$GRAFANA_CONF"
    # Security
    sed -i '/^\[security\]/,/^\[/ s/^;*\s*allow_embedding\s*=.*/allow_embedding = true/' "$GRAFANA_CONF"
    # Anonymous auth
    sed -i '/^\[auth.anonymous\]/,/^\[/ s/^;*\s*enabled\s*=.*/enabled = true/' "$GRAFANA_CONF"
    sed -i '/^\[auth.anonymous\]/,/^\[/ s/^;*\s*org_role\s*=.*/org_role = Viewer/' "$GRAFANA_CONF"
    ok "grafana.ini: allow_embedding=true, anonymous auth=Viewer, port=$GRAFANA_PORT"
fi

# 6b: Provision Prometheus data source (remove conflicting defaults first)
mkdir -p "$GRAFANA_PROV/datasources"
for f in "$GRAFANA_PROV/datasources"/*.yaml "$GRAFANA_PROV/datasources"/*.yml; do
    [[ -f "$f" ]] && [[ "$(basename "$f")" != "pillar.yml" ]] && rm -f "$f"
done
cat > "$GRAFANA_PROV/datasources/pillar.yml" <<EOF
apiVersion: 1
datasources:
  - name: Pillar Prometheus
    type: prometheus
    uid: pillar-prometheus
    access: proxy
    url: http://localhost:${PROMETHEUS_PORT}
    isDefault: true
    editable: false
EOF
ok "provisioned data source: Pillar Prometheus (uid: pillar-prometheus)"

# 6c: Provision dashboard directory
mkdir -p "$GRAFANA_PROV/dashboards"
cat > "$GRAFANA_PROV/dashboards/pillar.yml" <<EOF
apiVersion: 1
providers:
  - name: Pillar
    orgId: 1
    folder: Pillar
    type: file
    disableDeletion: false
    updateIntervalSeconds: 30
    options:
      path: ${GRAFANA_DASHBOARDS_DIR}
      foldersFromFilesStructure: false
EOF
ok "provisioned dashboard provider"

# 6d: Copy dashboard JSON files
mkdir -p "$GRAFANA_DASHBOARDS_DIR"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd 2>/dev/null || true)"
DASHBOARD_SRC=""
if [[ -n "$SCRIPT_DIR" && -d "$SCRIPT_DIR/../controller/dashboards/grafana" ]]; then
    DASHBOARD_SRC="$SCRIPT_DIR/../controller/dashboards/grafana"
fi

DASHBOARDS_COPIED=false
if [[ -n "$DASHBOARD_SRC" ]] && ls "$DASHBOARD_SRC"/*.json &>/dev/null; then
    cp "$DASHBOARD_SRC"/*.json "$GRAFANA_DASHBOARDS_DIR/"
    DASHBOARDS_COPIED=true
    ok "copied dashboard JSONs to $GRAFANA_DASHBOARDS_DIR ($(ls "$DASHBOARD_SRC"/*.json | wc -l) files)"
else
    info "dashboard JSONs not found locally — will fetch from controller API after startup"
fi

chown -R grafana:grafana "$GRAFANA_DASHBOARDS_DIR" 2>/dev/null || true
chown -R grafana:grafana "$GRAFANA_PROV" 2>/dev/null || true

# Start Grafana
systemctl daemon-reload
systemctl enable grafana-server 2>/dev/null
systemctl restart grafana-server
sleep 2

if systemctl is-active --quiet grafana-server; then
    ok "Grafana running on port $GRAFANA_PORT"
else
    fail "Grafana failed to start"
    journalctl -u grafana-server --no-pager -n 5
fi

# ==============================================================================
# Phase 7: Write controller config
# ==============================================================================

section "Writing controller configuration"

GRAFANA_URL="http://localhost:${GRAFANA_PORT}"
CONTROLLER_CONFIG="$CONFIG_DIR/controller.yaml"

if [[ -f "$CONTROLLER_CONFIG" ]]; then
    ok "config exists: $CONTROLLER_CONFIG (not overwriting)"
    # Ensure grafana_url is present
    if ! grep -q "grafana_url" "$CONTROLLER_CONFIG" 2>/dev/null; then
        echo "grafana_url: \"${GRAFANA_URL}\"" >> "$CONTROLLER_CONFIG"
        ok "appended grafana_url to existing config"
    fi
else
    cat > "$CONTROLLER_CONFIG" <<EOF
grpc_listen: "$GRPC_LISTEN"
http_listen: "$HTTP_LISTEN"
db_path: "$DB_PATH"
retention_days: $RETENTION_DAYS
external_url: "$EXTERNAL_URL"
grafana_url: "$GRAFANA_URL"
certs_dir: "/etc/pillar/certs"
EOF
    chown "$PILLAR_USER:$PILLAR_GROUP" "$CONTROLLER_CONFIG"
    chmod 644 "$CONTROLLER_CONFIG"
    ok "wrote $CONTROLLER_CONFIG"
fi

# ==============================================================================
# Phase 8: Controller systemd service
# ==============================================================================

section "Installing controller service"

UNIT_FILE="/etc/systemd/system/pillar-controller.service"
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

Environment=PILLAR_CONTROLLER_CONFIG=$CONTROLLER_CONFIG
Environment=RUST_LOG=info

StandardOutput=journal
StandardError=journal
SyslogIdentifier=pillar-controller

LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF
ok "wrote $UNIT_FILE"

systemctl daemon-reload
systemctl enable pillar-controller 2>/dev/null
systemctl restart pillar-controller
sleep 2

if systemctl is-active --quiet pillar-controller; then
    ok "pillar-controller is running"
else
    fail "pillar-controller failed to start"
    journalctl -u pillar-controller --no-pager -n 10
fi

# ==============================================================================
# Phase 9: Fetch dashboards from controller API (fallback)
# ==============================================================================

if [[ "$DASHBOARDS_COPIED" != "true" ]]; then
    section "Fetching dashboards from controller API"

    CONTROLLER_URL="http://localhost:${HTTP_PORT}"
    # Wait for controller to be ready (up to 15 seconds)
    for i in 1 2 3 4 5; do
        if curl -sf "$CONTROLLER_URL/api/overview" >/dev/null 2>&1; then
            break
        fi
        sleep 3
    done

    if curl -sf "$CONTROLLER_URL/api/dashboards/fleet-overview" -o "$GRAFANA_DASHBOARDS_DIR/fleet-overview.json" 2>/dev/null; then
        ok "fetched fleet-overview dashboard"
    else
        warn "could not fetch fleet-overview dashboard from controller"
    fi
    if curl -sf "$CONTROLLER_URL/api/dashboards/node-detail" -o "$GRAFANA_DASHBOARDS_DIR/node-detail.json" 2>/dev/null; then
        ok "fetched node-detail dashboard"
    else
        warn "could not fetch node-detail dashboard from controller"
    fi
    chown -R grafana:grafana "$GRAFANA_DASHBOARDS_DIR" 2>/dev/null || true
fi

# ==============================================================================
# Summary
# ==============================================================================

section "Installation complete"

echo ""
echo "  Pillar Controller running!"
echo ""
echo "  Controller:"
echo "    UI:       http://localhost:${HTTP_PORT}"
echo "    gRPC:     $GRPC_LISTEN"
echo "    Config:   $CONTROLLER_CONFIG"
echo "    Database: $DB_PATH"
if [[ -n "$EXTERNAL_URL" ]]; then
echo "    External: $EXTERNAL_URL"
fi
echo ""
echo "  Prometheus:"
echo "    URL:      http://localhost:${PROMETHEUS_PORT}"
echo "    Config:   ${PROM_CONFIG_DIR}/prometheus.yml"
echo "    Scraping: localhost:${HTTP_PORT}/metrics"
echo ""
echo "  Grafana:"
echo "    URL:      http://localhost:${GRAFANA_PORT}"
echo "    Embedded: http://localhost:${HTTP_PORT}/grafana"
echo "    Dashboards provisioned automatically"
echo ""
echo "  Add nodes:"
if [[ -n "$EXTERNAL_URL" ]]; then
echo "    curl -sSL ${S3_BASE}/latest/install-node.sh | sudo bash -s -- --controller $EXTERNAL_URL"
else
echo "    curl -sSL ${S3_BASE}/latest/install-node.sh | sudo bash -s -- --controller http://<this-ip>:50051"
fi
echo ""
echo "  Logs:"
echo "    journalctl -u pillar-controller -f"
echo "    journalctl -u prometheus -f"
echo "    journalctl -u grafana-server -f"
echo ""

if [[ $FAIL_COUNT -gt 0 ]]; then
    echo -e "  ${RED}$FAIL_COUNT check(s) failed — review above.${NC}"
elif [[ $WARN_COUNT -gt 0 ]]; then
    echo -e "  ${YELLOW}$WARN_COUNT warning(s) — review above.${NC}"
else
    echo -e "  ${GREEN}All checks passed.${NC}"
fi
echo ""
