#!/usr/bin/env bash
#
# Pillar Node installer — installs operator, link, Solana CLI, Rust toolchain,
# creates the sol user, applies sysctl tuning, and generates validator keypairs.
#
# Usage:
#   sudo ./install-node.sh --binaries-dir /path/to/binaries --controller-endpoint http://10.0.0.1:50051
#   sudo ./install-node.sh --binaries-dir /path/to/binaries --controller-endpoint http://10.0.0.1:50051 --node-id my-node
#   sudo ./install-node.sh --binaries-dir /path/to/binaries --controller-endpoint http://10.0.0.1:50051 --cluster testnet
#
# Idempotent — safe to run multiple times.

set -euo pipefail

# ------------------------------------------------------------------------------
# Defaults
# ------------------------------------------------------------------------------

SOL_USER="sol"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/pillar"
STATE_DIR="/var/run/pillar"
LOG_DIR="/var/log/pillar"

ROLE="rpc"
CLIENT="agave"
CLUSTER="mainnet-beta"
REFERENCE_RPC=""
CONTROLLER_ENDPOINT=""
NODE_ID=""
BINARIES_DIR=""
SOLANA_VERSION="stable"

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
        --binaries-dir)          BINARIES_DIR="$2";          shift 2 ;;
        --role)                  ROLE="$2";                  shift 2 ;;
        --client)                CLIENT="$2";                shift 2 ;;
        --cluster)               CLUSTER="$2";               shift 2 ;;
        --reference-rpc)         REFERENCE_RPC="$2";         shift 2 ;;
        --controller-endpoint)   CONTROLLER_ENDPOINT="$2";   shift 2 ;;
        --node-id)               NODE_ID="$2";               shift 2 ;;
        --solana-version)        SOLANA_VERSION="$2";        shift 2 ;;
        --help|-h)
            head -12 "$0" | tail -8
            exit 0
            ;;
        *)
            die "unknown argument: $1 (try --help)"
            ;;
    esac
done

# Cluster-aware reference RPC defaults (if user didn't override)
if [[ -z "$REFERENCE_RPC" ]]; then
    case "$CLUSTER" in
        devnet)       REFERENCE_RPC="https://api.devnet.solana.com" ;;
        testnet)      REFERENCE_RPC="https://api.testnet.solana.com" ;;
        *)            REFERENCE_RPC="https://api.mainnet-beta.solana.com" ;;
    esac
fi

if [[ -z "$BINARIES_DIR" ]]; then
    die "--binaries-dir is required (path to directory containing operator and link binaries)"
fi

if [[ ! -f "$BINARIES_DIR/operator" ]]; then
    die "operator binary not found in $BINARIES_DIR"
fi
if [[ ! -f "$BINARIES_DIR/link" ]]; then
    die "link binary not found in $BINARIES_DIR"
fi

if [[ -z "$CONTROLLER_ENDPOINT" ]]; then
    die "--controller-endpoint is required (e.g. http://10.0.0.1:50051)"
fi

# Default node_id to short hostname if not set
if [[ -z "$NODE_ID" ]]; then
    NODE_ID="$(hostname -s 2>/dev/null || echo "unknown")"
fi

# ------------------------------------------------------------------------------
# Phase 1: Preflight checks
# ------------------------------------------------------------------------------

section "Preflight checks"

if [[ $EUID -ne 0 ]]; then
    die "this script must be run as root (use sudo)"
fi

if [[ "$(uname -s)" != "Linux" ]]; then
    die "this script only supports Linux (detected: $(uname -s))"
fi
ok "Linux detected"

ARCH="$(uname -m)"
if [[ "$ARCH" != "x86_64" && "$ARCH" != "aarch64" ]]; then
    die "unsupported architecture: $ARCH (need x86_64 or aarch64)"
fi
ok "architecture: $ARCH"

if ! command -v systemctl &>/dev/null; then
    die "systemd not found — pillar requires systemd"
fi
ok "systemd available"

if [[ ! -d /proc/self ]]; then
    fail "/proc not mounted — system metrics will not work"
else
    ok "/proc available"
fi

# ------------------------------------------------------------------------------
# Phase 2: System assessment (cluster-aware thresholds)
# ------------------------------------------------------------------------------

section "System assessment"

# Set cluster-aware thresholds
case "$CLUSTER" in
    devnet)
        CPU_WARN=4;  CPU_OK=8
        RAM_WARN=8;  RAM_OK=16
        REQUIRED_MOUNTS="/mnt/ledger"
        ;;
    testnet)
        CPU_WARN=8;  CPU_OK=16
        RAM_WARN=32; RAM_OK=64
        REQUIRED_MOUNTS="/mnt/ledger /mnt/snapshots"
        ;;
    *)
        CPU_WARN=12; CPU_OK=16
        RAM_WARN=128; RAM_OK=256
        REQUIRED_MOUNTS="/mnt/ledger /mnt/snapshots /mnt/accounts"
        ;;
esac

# CPU check — hard fail below 4 cores
CPU_CORES=$(nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo 2>/dev/null || echo 0)
if [[ $CPU_CORES -lt 4 ]]; then
    die "at least 4 CPU cores required to run a validator (detected: $CPU_CORES)"
fi
if [[ $CPU_CORES -ge $CPU_OK ]]; then
    ok "CPU cores: $CPU_CORES"
elif [[ $CPU_CORES -ge $CPU_WARN ]]; then
    warn "CPU cores: $CPU_CORES (${CPU_OK}+ recommended for $CLUSTER)"
else
    fail "CPU cores: $CPU_CORES (minimum ${CPU_WARN} for $CLUSTER, ${CPU_OK}+ recommended)"
fi

# RAM check — hard fail below 8GB
RAM_KB=$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo 0)
RAM_GB=$((RAM_KB / 1024 / 1024))
if [[ $RAM_GB -lt 8 ]]; then
    die "at least 8GB RAM required to run a validator (detected: ${RAM_GB}GB)"
fi
if [[ $RAM_GB -ge $RAM_OK ]]; then
    ok "RAM: ${RAM_GB}GB"
elif [[ $RAM_GB -ge $RAM_WARN ]]; then
    warn "RAM: ${RAM_GB}GB (${RAM_OK}GB+ recommended for $CLUSTER)"
else
    fail "RAM: ${RAM_GB}GB (minimum ${RAM_WARN}GB for $CLUSTER, ${RAM_OK}GB+ recommended)"
fi

# CPU feature checks
if grep -q avx2 /proc/cpuinfo 2>/dev/null; then
    ok "CPU feature: AVX2 supported"
else
    warn "CPU feature: AVX2 not detected (required for prebuilt Solana binaries)"
fi

if grep -q sha_ni /proc/cpuinfo 2>/dev/null; then
    ok "CPU feature: SHA extensions supported"
else
    warn "CPU feature: SHA extensions not detected (recommended by Anza)"
fi

# Check disk mounts (cluster-aware)
for mount_point in /mnt/ledger /mnt/snapshots /mnt/accounts; do
    if mountpoint -q "$mount_point" 2>/dev/null; then
        DISK_TOTAL_KB=$(df -k "$mount_point" | tail -1 | awk '{print $2}')
        DISK_TOTAL_GB=$((DISK_TOTAL_KB / 1024 / 1024))
        ok "$mount_point: ${DISK_TOTAL_GB}GB"
    else
        # Check if this mount is required for the cluster
        if echo "$REQUIRED_MOUNTS" | grep -qw "$mount_point"; then
            warn "$mount_point not mounted (required for $CLUSTER)"
        else
            info "$mount_point not mounted (optional for $CLUSTER)"
        fi
    fi
done

# Network: can we reach the reference RPC?
if curl -sf --max-time 5 -o /dev/null "$REFERENCE_RPC"; then
    ok "network: can reach $REFERENCE_RPC"
else
    warn "network: cannot reach $REFERENCE_RPC"
fi

# Network bandwidth hint
PRIMARY_IFACE=$(ip route show default 2>/dev/null | awk '{print $5; exit}' || true)
if [[ -n "$PRIMARY_IFACE" ]] && command -v ethtool &>/dev/null; then
    LINK_SPEED=$(ethtool "$PRIMARY_IFACE" 2>/dev/null | grep Speed | awk '{print $2}' || true)
    if [[ -n "$LINK_SPEED" ]]; then
        SPEED_NUM=$(echo "$LINK_SPEED" | grep -oP '\d+' || true)
        if [[ -n "$SPEED_NUM" && "$SPEED_NUM" -lt 1000 ]]; then
            warn "network: link speed $LINK_SPEED on $PRIMARY_IFACE (1Gbps+ recommended)"
        else
            ok "network: link speed $LINK_SPEED on $PRIMARY_IFACE"
        fi
    fi
fi

info "Ensure TCP+UDP ports 8000-10000 are open for P2P (gossip, turbine, repair)"

# ------------------------------------------------------------------------------
# Phase 3: Create sol user and directories
# ------------------------------------------------------------------------------

section "Setting up sol user and directories"

if id "$SOL_USER" &>/dev/null; then
    ok "user $SOL_USER exists"
else
    useradd --system --create-home --home-dir /home/sol --shell /bin/bash "$SOL_USER"
    ok "created user $SOL_USER with home /home/sol"
fi

for dir in "$CONFIG_DIR" "$STATE_DIR" "$LOG_DIR"; do
    mkdir -p "$dir"
    chown "$SOL_USER:$SOL_USER" "$dir"
    chmod 755 "$dir"
done
ok "directories ready"

# Ensure sol owns the data directories
for mount_point in /mnt/ledger /mnt/snapshots /mnt/accounts; do
    if [[ -d "$mount_point" ]]; then
        chown "$SOL_USER:$SOL_USER" "$mount_point"
        ok "chown $SOL_USER:$SOL_USER $mount_point"
    fi
done

# ------------------------------------------------------------------------------
# Phase 3b: Sudoers for sol to manage validator systemd services
# ------------------------------------------------------------------------------

SUDOERS_FILE="/etc/sudoers.d/sol-systemctl"
cat > "$SUDOERS_FILE" <<'EOF'
# Allow sol user to manage systemd services without a password.
# Used by pillar-operator to start/stop/restart the validator.
sol ALL=(root) NOPASSWD: /usr/bin/systemctl
EOF
chmod 440 "$SUDOERS_FILE"
ok "wrote $SUDOERS_FILE"

# ------------------------------------------------------------------------------
# Phase 3c: Apply sysctl tuning (Anza-recommended)
# ------------------------------------------------------------------------------

section "Applying sysctl tuning"

SYSCTL_CONF="/etc/sysctl.d/21-agave-validator.conf"
cat > "$SYSCTL_CONF" <<EOF
# Anza-recommended sysctl settings for Solana validators
net.core.rmem_max = 134217728
net.core.wmem_max = 134217728
vm.max_map_count = 1000000
fs.nr_open = 1000000
EOF
sysctl -p "$SYSCTL_CONF" >/dev/null 2>&1 || warn "failed to apply sysctl settings (may require reboot)"
ok "wrote $SYSCTL_CONF"

LIMITS_CONF="/etc/security/limits.d/sol-nofile.conf"
cat > "$LIMITS_CONF" <<EOF
# Solana validator file descriptor limits
sol soft nofile 1000000
sol hard nofile 1000000
EOF
ok "wrote $LIMITS_CONF"

# ------------------------------------------------------------------------------
# Phase 3d: Install Solana CLI
# ------------------------------------------------------------------------------

section "Solana CLI"

if su - sol -c "command -v agave-validator" &>/dev/null; then
    EXISTING_VER=$(su - sol -c "agave-validator --version 2>/dev/null | head -1" || echo "unknown")
    ok "Solana CLI already installed ($EXISTING_VER)"
else
    info "Installing Solana CLI (version: $SOLANA_VERSION) as sol user..."
    if su - sol -c "sh -c \"\$(curl -sSfL https://release.anza.xyz/${SOLANA_VERSION}/install)\"" 2>&1; then
        # Ensure Solana bin is in sol's PATH via .profile
        SOL_PROFILE="/home/sol/.profile"
        SOLANA_PATH_LINE='export PATH="/home/sol/.local/share/solana/install/active_release/bin:$PATH"'
        if ! grep -qF "$SOLANA_PATH_LINE" "$SOL_PROFILE" 2>/dev/null; then
            echo "$SOLANA_PATH_LINE" >> "$SOL_PROFILE"
        fi
        ok "Solana CLI installed"
    else
        warn "Solana CLI install failed — you can install manually later as the sol user"
    fi
fi

# ------------------------------------------------------------------------------
# Phase 3e: Install Rust toolchain (required for building agave v3+)
# ------------------------------------------------------------------------------

section "Rust toolchain"

AGAVE_RUST_VERSION="1.86.0"

if su - sol -c "command -v rustc" &>/dev/null; then
    EXISTING_RUST=$(su - sol -c "rustc --version 2>/dev/null" || echo "unknown")
    ok "Rust already installed ($EXISTING_RUST)"
    # Ensure the required version is available
    if ! su - sol -c ". \$HOME/.cargo/env && rustup toolchain list 2>/dev/null" 2>/dev/null | grep -q "$AGAVE_RUST_VERSION"; then
        info "Installing Rust $AGAVE_RUST_VERSION toolchain (required for agave v3)..."
        if su - sol -c ". \$HOME/.cargo/env && rustup install $AGAVE_RUST_VERSION" 2>&1; then
            ok "Rust $AGAVE_RUST_VERSION installed"
        else
            warn "failed to install Rust $AGAVE_RUST_VERSION — you can install manually: rustup install $AGAVE_RUST_VERSION"
        fi
    else
        ok "Rust $AGAVE_RUST_VERSION toolchain available"
    fi
else
    info "Installing Rust toolchain as sol user..."
    if su - sol -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain '"$AGAVE_RUST_VERSION" 2>&1; then
        ok "Rust $AGAVE_RUST_VERSION installed"
    else
        warn "Rust install failed — you can install manually as the sol user"
    fi
fi

# Ensure cargo bin is in sol's PATH
SOL_CARGO_PATH='export PATH="$HOME/.cargo/bin:$PATH"'
SOL_PROFILE="/home/sol/.profile"
if ! grep -qF "$SOL_CARGO_PATH" "$SOL_PROFILE" 2>/dev/null; then
    echo "$SOL_CARGO_PATH" >> "$SOL_PROFILE"
    ok "added cargo to sol PATH in .profile"
fi

# ------------------------------------------------------------------------------
# Phase 3f: Generate validator keypairs
# ------------------------------------------------------------------------------

section "Validator keypairs"

SOLANA_KEYGEN=""
if su - sol -c "command -v solana-keygen" &>/dev/null; then
    SOLANA_KEYGEN="solana-keygen"
fi

for keypair_name in validator-keypair vote-account-keypair authorized-withdrawer-keypair; do
    KEYPAIR_PATH="/home/sol/${keypair_name}.json"
    if [[ -f "$KEYPAIR_PATH" ]]; then
        ok "$keypair_name exists: $KEYPAIR_PATH"
    elif [[ -n "$SOLANA_KEYGEN" ]]; then
        if su - sol -c "solana-keygen new --no-bip39-passphrase -o $KEYPAIR_PATH" &>/dev/null; then
            chmod 600 "$KEYPAIR_PATH"
            chown sol:sol "$KEYPAIR_PATH"
            ok "generated $keypair_name: $KEYPAIR_PATH"
        else
            warn "failed to generate $keypair_name"
        fi
    else
        info "$keypair_name not found and solana-keygen not available — generate manually after installing Solana CLI"
    fi
done

if [[ -f "/home/sol/authorized-withdrawer-keypair.json" ]]; then
    echo ""
    warn "IMPORTANT: Back up /home/sol/authorized-withdrawer-keypair.json to a secure offline location!"
    warn "If lost, you cannot change the withdraw authority on your vote account."
    echo ""
fi

# ------------------------------------------------------------------------------
# Phase 4: Install binaries
# ------------------------------------------------------------------------------

section "Installing binaries"

for name in operator link; do
    DST="$INSTALL_DIR/$name"
    install -m 755 "$BINARIES_DIR/$name" "$DST"
    ok "installed $name -> $DST"
done

# ------------------------------------------------------------------------------
# Phase 5: Write config files (only if they don't exist)
# ------------------------------------------------------------------------------

section "Writing configuration"

OPERATOR_CONFIG="$CONFIG_DIR/operator.yaml"
if [[ -f "$OPERATOR_CONFIG" ]]; then
    ok "operator config exists: $OPERATOR_CONFIG (not overwriting)"
else
    cat > "$OPERATOR_CONFIG" <<EOF
role: $ROLE
client: $CLIENT
state_path: $STATE_DIR/operator-state.bin
network:
  cluster: $CLUSTER
  reference_rpc_urls:
    - $REFERENCE_RPC
lifecycle:
  service_name: solana-validator
  max_startup_wait_secs: 600
  max_catchup_wait_secs: 1800
  crash_window_secs: 3600
  crash_threshold: 3
health:
  check_interval_secs: 20
  slots_behind_threshold: 100
  rpc_timeout_secs: 10
  local_rpc_url: "http://127.0.0.1:8899"
  consecutive_off_threshold: 3
snapshot:
  download_method: tcp
  server_hostname: ""
  staleness_threshold_slots: 1000
  download_timeout_secs: 3600
paths:
  ledger_path: /mnt/ledger
  snapshot_path: /mnt/snapshots
EOF
    chown "$SOL_USER:$SOL_USER" "$OPERATOR_CONFIG"
    chmod 644 "$OPERATOR_CONFIG"
    ok "wrote $OPERATOR_CONFIG"
fi

LINK_CONFIG="$CONFIG_DIR/link.yaml"
if [[ -f "$LINK_CONFIG" ]]; then
    ok "link config exists: $LINK_CONFIG (not overwriting)"
else
    cat > "$LINK_CONFIG" <<EOF
state_path: $STATE_DIR/operator-state.bin
poll_interval_secs: 5
http_listen: "0.0.0.0:9090"
controller:
  endpoint: "$CONTROLLER_ENDPOINT"
  node_id: "$NODE_ID"
  report_interval_secs: 10
EOF
    chown "$SOL_USER:$SOL_USER" "$LINK_CONFIG"
    chmod 644 "$LINK_CONFIG"
    ok "wrote $LINK_CONFIG"
fi

# ------------------------------------------------------------------------------
# Phase 6: Systemd services
# ------------------------------------------------------------------------------

section "Installing systemd services"

write_service() {
    local name="$1"
    local desc="$2"
    local env_var="$3"
    local config_file="$4"
    local extra_after="${5:-}"
    local unit_file="/etc/systemd/system/${name}.service"

    cat > "$unit_file" <<EOF
[Unit]
Description=Pillar $desc
After=network-online.target${extra_after}
Wants=network-online.target

[Service]
Type=simple
User=$SOL_USER
Group=$SOL_USER
ExecStart=$INSTALL_DIR/$name
Restart=always
RestartSec=5

Environment=${env_var}=${config_file}
Environment=RUST_LOG=info

StandardOutput=journal
StandardError=journal
SyslogIdentifier=$name

RuntimeDirectory=pillar
RuntimeDirectoryMode=0755

LimitNOFILE=1000000

[Install]
WantedBy=multi-user.target
EOF
    ok "wrote $unit_file"
}

write_service "operator" "Operator" "PILLAR_CONFIG" "$OPERATOR_CONFIG"
write_service "link" "Link" "PILLAR_LINK_CONFIG" "$LINK_CONFIG" " operator.service"

systemctl daemon-reload
systemctl enable operator link 2>/dev/null
ok "services enabled"

# Start services
systemctl restart operator
sleep 1
systemctl restart link
sleep 2

for svc in operator link; do
    if systemctl is-active --quiet "$svc" 2>/dev/null; then
        ok "$svc is running"
    else
        fail "$svc failed to start"
        journalctl -u "$svc" --no-pager -n 10
    fi
done

# ------------------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------------------

section "Installation complete"

echo ""
echo "  Binaries:   $INSTALL_DIR/operator, $INSTALL_DIR/link"
echo "  Config:     $OPERATOR_CONFIG, $LINK_CONFIG"
echo "  State:      $STATE_DIR/operator-state.bin"
echo "  Services:   operator.service, link.service"
echo "  Controller: $CONTROLLER_ENDPOINT"
echo "  Node ID:    $NODE_ID"
echo "  Sol user:   /home/sol"
echo "  Keypairs:   /home/sol/validator-keypair.json"
echo "              /home/sol/vote-account-keypair.json"
echo "              /home/sol/authorized-withdrawer-keypair.json"
echo "  Sudoers:    $SUDOERS_FILE"
echo "  Sysctl:     $SYSCTL_CONF"
echo "  Limits:     $LIMITS_CONF"
echo ""

if [[ $FAIL_COUNT -gt 0 ]]; then
    echo -e "  ${RED}$FAIL_COUNT check(s) failed — review above.${NC}"
elif [[ $WARN_COUNT -gt 0 ]]; then
    echo -e "  ${YELLOW}$WARN_COUNT warning(s) — review above.${NC}"
else
    echo -e "  ${GREEN}All checks passed.${NC}"
fi

echo ""
echo "  Commands:"
echo "    journalctl -u operator -f"
echo "    journalctl -u link -f"
echo "    curl http://localhost:9090/health"
echo "    curl http://localhost:9090/status"
echo ""
