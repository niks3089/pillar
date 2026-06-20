#!/bin/bash
# Run multiple Surfpool validators on a single host as separate Pillar fleet nodes.
#
# Pillar's provisioning UI is one-validator-per-host, so to demo a multi-validator fleet
# on one box this script clones the existing agent config + creates an extra surfpool
# service + pillar-agent instance per entry (each with a unique node_id, RPC port, and
# agent HTTP port). The agent reports each surfpool node as healthy (its reference RPC is
# pointed at its own local surfpool RPC).
#
# Prereqs: a base /etc/pillar/agent.yaml (from install-node.sh) and surfpool at
# /usr/local/bin/surfpool. Run as root on the validator host.
#
# Edit the instances at the bottom, then: sudo bash setup-surfpool-fleet.sh
set -e

setup_instance() {
  local NAME=$1 RPC=$2 WS=$3 HTTP=$4 NET=$5 CLUSTER=$6
  install -d -o sol -g sol "/home/sol/surfpool-$NAME"

  cat > "/etc/systemd/system/surfpool-$NAME.service" <<UNIT
[Unit]
Description=Surfpool ($NAME) local Solana test validator
After=network-online.target
Wants=network-online.target
[Service]
Type=simple
User=sol
WorkingDirectory=/home/sol/surfpool-$NAME
ExecStart=/usr/local/bin/surfpool start --no-tui --no-studio --host 0.0.0.0 --port $RPC --ws-port $WS --network $NET
Restart=on-failure
RestartSec=2
LimitNOFILE=1000000
[Install]
WantedBy=multi-user.target
UNIT

  # Clone the base agent config, repointing node_id / ports / reference RPC / service.
  cp /etc/pillar/agent.yaml "/etc/pillar/agent-$NAME.yaml"
  sed -i \
    -e "s|127.0.0.1:8899|127.0.0.1:$RPC|g" \
    -e "s|0.0.0.0:9090|0.0.0.0:$HTTP|" \
    -e "s|node_id: \"ubuntu-server\"|node_id: \"surfpool-$NAME\"|" \
    -e "s|cluster: devnet|cluster: $CLUSTER|" \
    -e "s|^  service_name: surfpool$|  service_name: surfpool-$NAME|" \
    -e "s|- solana-validator.service|- surfpool-$NAME.service|" \
    -e "s|- pillar-agent.service|- pillar-agent-$NAME.service|" \
    "/etc/pillar/agent-$NAME.yaml"

  # Clone the agent unit, pointing at the new config + a unique runtime dir.
  sed \
    -e "s|PILLAR_AGENT_CONFIG=/etc/pillar/agent.yaml|PILLAR_AGENT_CONFIG=/etc/pillar/agent-$NAME.yaml|" \
    -e "s|SyslogIdentifier=pillar-agent|SyslogIdentifier=pillar-agent-$NAME|" \
    -e "s|RuntimeDirectory=pillar$|RuntimeDirectory=pillar-$NAME|" \
    -e "s|Description=.*|Description=Pillar Agent ($NAME)|" \
    /etc/systemd/system/pillar-agent.service > "/etc/systemd/system/pillar-agent-$NAME.service"

  systemctl daemon-reload
  systemctl enable --now "surfpool-$NAME" "pillar-agent-$NAME"
  echo "$NAME: surfpool=$(systemctl is-active surfpool-$NAME) agent=$(systemctl is-active pillar-agent-$NAME)"
}

#                NAME     RPC   WS    HTTP  NETWORK   CLUSTER
setup_instance   mainnet  8901  8902  9091  mainnet   mainnet-beta
setup_instance   testnet  8903  8904  9092  testnet   testnet
echo "DONE"
